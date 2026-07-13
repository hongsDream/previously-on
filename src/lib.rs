pub mod app_server;
pub mod config;
pub mod context_pack;
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
