//! IsoFoundry core: platform-independent pieces shared by the GUI, the CLI
//! and the elevated helper.

pub mod error;
pub mod fsutil;
pub mod ipc;
pub mod job;
pub mod log;
pub mod paths;
pub mod report;
pub mod settings;
pub mod strings;
pub mod tasks;

/// Application version, taken from the workspace manifest.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Product name used in titles, paths and reports.
pub const APP_NAME: &str = "IsoFoundry";
