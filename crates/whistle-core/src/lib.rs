//! Everything Whistle does that isn't argument parsing or printing, so the CLI
//! and the desktop app share one implementation.

pub mod backend;
pub mod fetch;
pub mod host;
pub mod index;
pub mod install;
pub mod paths;
pub mod recipe;
pub mod resolve;
pub mod runner;
pub mod state;
pub mod wine;
