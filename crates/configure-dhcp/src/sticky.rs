//! Sticky state: stacks previously detected (survive reboot under /data).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

const DEFAULT_STATE_PATH: &str = "/data/etc/configure-dhcp.state";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StickyState {
    /// Stack names that previously gated DHCP on.
    pub stacks: BTreeSet<String>,
}

impl StickyState {
    #[must_use]
    pub fn path_default() -> PathBuf {
        PathBuf::from(DEFAULT_STATE_PATH)
    }

    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => {
                let s: Self = serde_json::from_str(&text)?;
                Ok(s)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(Error::io_at(path, e)),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::io_at(parent, e))?;
        }
        let text = serde_json::to_string_pretty(self)?;
        fs::write(path, text).map_err(|e| Error::io_at(path, e))?;
        Ok(())
    }

    #[must_use]
    pub fn has_any(&self) -> bool {
        !self.stacks.is_empty()
    }

    pub fn remember(&mut self, stack_name: &str) {
        self.stacks.insert(stack_name.to_string());
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use tempfile::tempdir;

    #[test]
    fn roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut s = StickyState::default();
        s.remember("omada");
        s.save(&path).unwrap();
        let loaded = StickyState::load(&path).unwrap();
        assert!(loaded.stacks.contains("omada"));
    }
}
