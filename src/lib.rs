pub mod app_server;
mod bounded_io;
pub mod config;
pub mod context_pack;
pub mod contracts;
pub mod domain;
pub mod git;
pub mod hook;
pub mod mcp;
pub mod redaction;
pub mod server;
pub mod setup;
pub mod store;

pub const APP_NAME: &str = "PreviouslyOn";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
