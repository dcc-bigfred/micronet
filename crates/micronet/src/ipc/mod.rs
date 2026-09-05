//! Unix control socket: 4-byte LE length + JSON.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use bigfred_shared_daemon::ipc::{
    read_frame_bytes, write_frame_with_limit, AcceptPolicy, Auth, BindError, BindOptions, Command,
    Connection, ErrorHandler, IpcError, RejectReason, Router, SessionMode,
};
use serde_json::Value;

use crate::apply::Status;
use crate::config::Config;
use crate::constants::{MAX_IPC_CLIENTS, MAX_IPC_FRAME_BYTES};
use crate::datadir;
use crate::error::{Error, Result};
use crate::version;

pub mod protocol;
pub use protocol::{Health, Request, Response};

/// Default `$DATA_DIR/run/micronet.sock`.
#[must_use]
pub fn default_socket() -> PathBuf {
    datadir::path(["run", "micronet.sock"])
}

/// Shared daemon snapshot for IPC.
pub struct Shared {
    pub config: RwLock<Config>,
    pub status: RwLock<Status>,
    pub applying: AtomicBool,
    pub health: RwLock<Health>,
}

impl Shared {
    #[must_use]
    pub fn new(config: Config, status: Status) -> Arc<Self> {
        Arc::new(Self {
            config: RwLock::new(config),
            status: RwLock::new(status),
            applying: AtomicBool::new(false),
            health: RwLock::new(Health::ok()),
        })
    }
}

/// Events the daemon loop must handle (from IPC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcEvent {
    Reconfigure,
}

struct IpcState {
    shared: Arc<Shared>,
    events: Sender<IpcEvent>,
}

pub fn write_frame_to(writer: &mut impl Write, msg: &impl serde::Serialize) -> Result<()> {
    write_frame_with_limit(writer, msg, MAX_IPC_FRAME_BYTES).map_err(map_frame)
}

pub fn read_frame_from(reader: &mut impl Read) -> Result<Vec<u8>> {
    read_frame_bytes(reader, MAX_IPC_FRAME_BYTES).map_err(map_frame)
}

fn map_frame(e: bigfred_shared_daemon::ipc::FrameError) -> Error {
    Error::Ipc(e.to_string())
}

fn map_bind(e: BindError) -> Error {
    match e {
        BindError::AlreadyRunning {
            process_name,
            location,
            ..
        } => Error::Ipc(format!("{process_name} already running at {location}")),
        BindError::Io { path, source } => Error::io_at(path, source),
    }
}

struct StatusCmd;
struct InfoCmd;
struct ReconfigureCmd;

impl Command<IpcState> for StatusCmd {
    fn name(&self) -> &'static str {
        "status"
    }
    fn execute(
        &self,
        state: &IpcState,
        _body: Value,
        conn: &mut Connection,
    ) -> std::result::Result<(), IpcError> {
        let resp = match (state.shared.status.read(), state.shared.health.read()) {
            (Ok(s), Ok(h)) => Response::from_status(&s, &h),
            _ => Response::Error {
                message: "status lock poisoned".into(),
            },
        };
        conn.reply(&resp).map_err(IpcError::from)
    }
}

impl Command<IpcState> for InfoCmd {
    fn name(&self) -> &'static str {
        "info"
    }
    fn execute(
        &self,
        _state: &IpcState,
        _body: Value,
        conn: &mut Connection,
    ) -> std::result::Result<(), IpcError> {
        conn.reply(&Response::from_info(&version::info()))
            .map_err(IpcError::from)
    }
}

impl Command<IpcState> for ReconfigureCmd {
    fn name(&self) -> &'static str {
        "reconfigure"
    }
    fn execute(
        &self,
        state: &IpcState,
        _body: Value,
        conn: &mut Connection,
    ) -> std::result::Result<(), IpcError> {
        let _ = state.events.send(IpcEvent::Reconfigure);
        conn.reply(&Response::Ok).map_err(IpcError::from)
    }
}

struct MicronetHooks;

impl ErrorHandler<IpcState> for MicronetHooks {
    fn unknown(&self, _state: &IpcState, type_name: &str, _body: &Value, conn: &mut Connection) {
        let _ = conn.reply(&Response::Error {
            message: format!(
                "unknown variant `{type_name}`, expected one of `status`, `info`, `reconfigure`"
            ),
        });
    }
    fn error(&self, _state: &IpcState, err: &IpcError, conn: &mut Connection) {
        let _ = conn.reply(&Response::Error {
            message: err.to_string(),
        });
    }
    fn reject(&self, _state: &IpcState, reason: RejectReason, _conn: &mut Connection) {
        if reason == RejectReason::Busy {
            log::warn!("ipc client limit {MAX_IPC_CLIENTS} reached");
        }
    }
}

/// Bind the control socket and serve requests in a background thread.
pub fn serve(path: &Path, shared: Arc<Shared>, events: Sender<IpcEvent>) -> Result<()> {
    let mut router = Router::new();
    router
        .add(StatusCmd)
        .map_err(|e| Error::Other(e.to_string()))?;
    router
        .add(InfoCmd)
        .map_err(|e| Error::Other(e.to_string()))?;
    router
        .add(ReconfigureCmd)
        .map_err(|e| Error::Other(e.to_string()))?;

    let state = Arc::new(IpcState { shared, events });
    bigfred_shared_daemon::ipc::serve_background(
        BindOptions {
            path: path.to_path_buf(),
            mode: 0o600,
            chown: None,
            process_name: "micronet",
        },
        AcceptPolicy {
            auth: Auth::None,
            session: SessionMode::OneShot,
            max_clients: Some(MAX_IPC_CLIENTS),
            max_frame: MAX_IPC_FRAME_BYTES,
        },
        router,
        MicronetHooks,
        state,
    )
    .map_err(map_bind)?;
    log::info!("ctl listening on {}", path.display());
    Ok(())
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
