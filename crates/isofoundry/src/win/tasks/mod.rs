//! The task windows. Each module owns one window class and opens a
//! modeless window owned by the main window; the operations
//! themselves are the same engine calls the CLI uses.

pub mod audioconv;
pub mod backupwin;
pub mod bootusb;
pub mod browse;
pub mod burnaudio;
pub mod burnfiles;
pub mod burnimage;
pub mod checksum;
pub mod convert;
pub mod copydisc;
pub mod copyimage;
pub mod create;
pub mod driveinfo;
pub mod drivepick;
pub mod edit;
pub mod erasewin;
pub mod recoverwin;
pub mod ripaudio;
pub mod surfacetest;
pub mod usbread;
pub mod usbtools;
pub mod winusb;
