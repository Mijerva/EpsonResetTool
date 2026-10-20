//! Reads and resets the waste ink counters of an Epson ET-2870 series printer
//! over USB, through the printer driver Windows already has.

pub mod app;
pub mod ctrl;
pub mod d4;
#[cfg(windows)]
pub mod fetch;
#[cfg(test)]
mod golden;
#[cfg(windows)]
pub mod gui;
pub mod log;
pub mod model;
pub mod packages;
pub mod printer;
pub mod reset;
#[cfg(any(test, feature = "demo"))]
pub mod sim;
pub mod store;
pub mod text;
#[cfg(windows)]
pub mod usb;
