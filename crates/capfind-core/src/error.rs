//! Error types for capfind-core.

use std::io;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("bincode (de)serialization error: {0}")]
    Bincode(#[from] bincode::Error),

    #[error("invalid index file: bad magic (got {got:?}, expected \"CAPFIND\\0\")")]
    BadMagic { got: [u8; 8] },

    #[error("unsupported index version {found} (this build understands {supported})")]
    UnsupportedVersion { found: u16, supported: u16 },

    #[error("truncated index file: needed {needed} bytes at offset {offset}, got {got}")]
    Truncated {
        offset: usize,
        needed: usize,
        got: usize,
    },

    #[error("decompression failed: {0}")]
    Decompress(String),
}

pub type Result<T> = std::result::Result<T, IndexError>;
