//! SIGTERM / SIGINT via `signal-hook` (no `unsafe` in this crate).

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::flag;

use crate::error::{Error, Result};

/// Install handlers that set `flag` on SIGTERM / SIGINT.
pub fn install(flag: &Arc<AtomicBool>) -> Result<()> {
    flag::register(SIGTERM, Arc::clone(flag)).map_err(|e| Error::Other(e.to_string()))?;
    flag::register(SIGINT, Arc::clone(flag)).map_err(|e| Error::Other(e.to_string()))?;
    Ok(())
}
