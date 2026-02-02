//! TUI module for interactive terminal interface

mod app;
pub mod theme;
pub mod widgets;

pub use app::{run, TuiConfig};
