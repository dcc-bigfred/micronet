//! Unix control socket: 4-byte LE length + JSON.

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use std::thread;

use crate::apply::Status;
use crate::config::Config;
use crate::constants::{MAX_IPC_CLIENTS, MAX_IPC_FRAME_BYTES};
use crate::datadir;
use crate::error::{Error, Result};
use crate::version;

pub mod protocol;
pub use protocol::{Request, Response};

/// Default `$DATA_DIR/run/micronet.sock`.
#[must_use]
pub fn default_socket() -> PathBuf {
    datadir::path(["run", "micronet.sock"])
}

/// Shared daemon snapshot for IPC.
pub struct Shared {
    pub config: RwLock<Config>,
    pub status: RwLock<Status>,
}

impl Shared {
    #[must_use]
    pub fn new(config: Config, status: Status) -> Arc<Self> {
        Arc::new(Self {
            config: RwLock::new(config),
            status: RwLock::new(status),
        })
    }
}

/// Events the daemon loop must handle (from IPC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcEvent {
    Reconfigure,
}

pub fn write_frame_to(writer: &mut impl Write, msg: &impl serde::Serialize) -> Result<()> {
    let payload = serde_json::to_vec(msg)?;
    if payload.len() > MAX_IPC_FRAME_BYTES {
        return Err(Error::Ipc(format!(
            "frame length {} exceeds max {MAX_IPC_FRAME_BYTES}",
            payload.len()
        )));
    }
    let len = u32::try_from(payload.len())
        .map_err(|_| Error::Ipc("frame too large for u32 length prefix".into()))?
        .to_le_bytes();
    writer.write_all(&len)?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

pub fn read_frame_from(reader: &mut impl Read) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_IPC_FRAME_BYTES {
        return Err(Error::Ipc(format!("frame length {len} too large")));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    Ok(buf)
}

fn bind_singleton(socket_path: &Path) -> Result<UnixListener> {
    match UnixStream::connect(socket_path) {
        Ok(stream) => {
            let pid = peer_pid(&stream);
            let where_ = if pid != 0 {
                format!("{} (pid {pid})", socket_path.display())
            } else {
                socket_path.display().to_string()
            };
            return Err(Error::Ipc(format!("micronet already running at {where_}")));
        }
        Err(e) if is_stale_socket_connect_error(&e) => {}
        Err(e) => return Err(Error::io_at(socket_path, e)),
    }
    match std::fs::remove_file(socket_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(Error::io_at(socket_path, e)),
    }
    UnixListener::bind(socket_path).map_err(|e| Error::io_at(socket_path, e))
}

fn is_stale_socket_connect_error(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
    )
}

fn peer_pid(stream: &UnixStream) -> u32 {
    use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
    getsockopt(stream, PeerCredentials)
        .map(|c| c.pid() as u32)
        .unwrap_or(0)
}

fn apply_socket_perms(socket_path: &Path) -> Result<()> {
    let mut perms = std::fs::metadata(socket_path)
        .map_err(|e| Error::io_at(socket_path, e))?
        .permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(socket_path, perms).map_err(|e| Error::io_at(socket_path, e))?;
    Ok(())
}

/// Bind the control socket and serve requests in a background thread.
pub fn serve(path: &Path, shared: Arc<Shared>, events: Sender<IpcEvent>) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io_at(parent, e))?;
        }
    }
    let listener = bind_singleton(path)?;
    apply_socket_perms(path)?;
    log::info!("ctl listening on {}", path.display());

    let path = path.to_path_buf();
    let clients = Arc::new(AtomicUsize::new(0));
    thread::Builder::new()
        .name("ctl".into())
        .spawn(move || {
            for conn in listener.incoming() {
                match conn {
                    Ok(stream) => {
                        let n = clients.load(Ordering::SeqCst);
                        if n >= MAX_IPC_CLIENTS {
                            log::warn!("ipc client limit {MAX_IPC_CLIENTS} reached");
                            drop(stream);
                            continue;
                        }
                        clients.fetch_add(1, Ordering::SeqCst);
                        let shared = Arc::clone(&shared);
                        let events = events.clone();
                        let clients = Arc::clone(&clients);
                        thread::spawn(move || {
                            handle_conn(stream, &shared, &events);
                            clients.fetch_sub(1, Ordering::SeqCst);
                        });
                    }
                    Err(_) => {
                        if !path.exists() {
                            break;
                        }
                    }
                }
            }
        })
        .map_err(|e| Error::Other(e.to_string()))?;
    Ok(())
}

fn handle_conn(mut stream: UnixStream, shared: &Shared, events: &Sender<IpcEvent>) {
    let payload = match read_frame_from(&mut stream) {
        Ok(p) => p,
        Err(e) => {
            log::debug!("ipc read: {e}");
            return;
        }
    };
    let req: Request = match serde_json::from_slice(&payload) {
        Ok(r) => r,
        Err(e) => {
            let _ = write_frame_to(
                &mut stream,
                &Response::Error {
                    message: e.to_string(),
                },
            );
            return;
        }
    };
    let resp = match req {
        Request::Status => match shared.status.read() {
            Ok(s) => Response::from_status(&s),
            Err(_) => Response::Error {
                message: "status lock poisoned".into(),
            },
        },
        Request::Info => Response::from_info(&version::info()),
        Request::Reconfigure => {
            let _ = events.send(IpcEvent::Reconfigure);
            Response::Ok
        }
    };
    let _ = write_frame_to(&mut stream, &resp);
}

/// Client: send one request, read one response.
pub fn call(socket: &Path, req: &Request) -> Result<Response> {
    let mut stream = UnixStream::connect(socket).map_err(|e| Error::io_at(socket, e))?;
    write_frame_to(&mut stream, req)?;
    let payload = read_frame_from(&mut stream)?;
    Ok(serde_json::from_slice(&payload)?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use std::io::Cursor;

    #[test]
    fn frame_roundtrip() {
        let mut buf = Vec::new();
        write_frame_to(&mut buf, &Request::Status).unwrap();
        let mut cur = Cursor::new(buf);
        let payload = read_frame_from(&mut cur).unwrap();
        let req: Request = serde_json::from_slice(&payload).unwrap();
        assert_eq!(req, Request::Status);
    }

    #[test]
    fn oversized_frame_rejected() {
        let mut too_big = (MAX_IPC_FRAME_BYTES as u32 + 1).to_le_bytes().to_vec();
        too_big.extend_from_slice(&[0u8; 8]);
        let mut cur = Cursor::new(too_big);
        assert!(read_frame_from(&mut cur).is_err());
    }
}
