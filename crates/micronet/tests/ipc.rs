//! IPC framing + live Unix socket.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use micronet::apply::Status;
use micronet::config::Config;
use micronet::ipc::{self, call, read_frame_from, write_frame_to, Request, Response, Shared};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp_sock() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("micronet-ipc-{nanos}-{seq}.sock"))
}

#[test]
fn oversized_frame_rejected() {
    let len = u32::try_from(micronet::constants::MAX_IPC_FRAME_BYTES)
        .unwrap_or(u32::MAX)
        .saturating_add(1);
    let mut too_big = len.to_le_bytes().to_vec();
    too_big.extend_from_slice(&[0u8; 8]);
    let mut cur = std::io::Cursor::new(too_big);
    assert!(read_frame_from(&mut cur).is_err());
}

#[test]
fn status_over_socket() {
    let sock = tmp_sock();
    let mut st = Status::empty();
    st.mode = micronet::apply::Mode::Gateway;
    st.iface = "eth0".into();
    st.cidr = Some("10.0.10.1/24".into());
    let shared = Shared::new(Config::default(), st);
    let (tx, _rx) = mpsc::channel();
    ipc::serve(&sock, shared, tx).expect("serve");
    std::thread::sleep(std::time::Duration::from_millis(50));
    let resp = call(&sock, &Request::Status).expect("call");
    match resp {
        Response::Status {
            iface, cidr, mode, ..
        } => {
            assert_eq!(iface, "eth0");
            assert_eq!(cidr.as_deref(), Some("10.0.10.1/24"));
            assert_eq!(mode, micronet::apply::Mode::Gateway);
        }
        other => panic!("unexpected {other:?}"),
    }
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn write_read_request() {
    let mut buf = Vec::new();
    write_frame_to(&mut buf, &Request::Reconfigure).unwrap();
    let mut cur = std::io::Cursor::new(buf);
    let payload = read_frame_from(&mut cur).unwrap();
    let req: Request = serde_json::from_slice(&payload).unwrap();
    assert_eq!(req, Request::Reconfigure);
    let _ = UnixStream::pair();
}
