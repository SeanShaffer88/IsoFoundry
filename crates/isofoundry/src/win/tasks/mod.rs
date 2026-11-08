//! The M1 task windows. Each module owns one window class and opens a
//! modeless window owned by the main window; the operations themselves are
//! the same `if-image`/`if-hash` calls the CLI uses (see `cli_image.rs`).

pub mod audioconv;
pub mod bootusb;
pub mod browse;
pub mod checksum;
pub mod convert;
pub mod create;
pub mod edit;
pub mod usbtools;
pub mod winusb;
