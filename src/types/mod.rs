//! Type definitions for toktrack
#![allow(unused_imports)]

mod error;
mod usage;

pub use error::*;
pub use usage::*;

/// Cache loading warning types
#[derive(Debug, Clone)]
#[allow(dead_code)] // String fields reserved for TUI display
pub enum CacheWarning {
    /// Failed to open or read cache file
    LoadFailed(String),
    /// Cache file was corrupted (invalid JSON)
    Corrupted(String),
}
