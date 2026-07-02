//! Host/device-control ability handlers.
//!
//! These handlers expose local device operations as daemon-owned abilities.
//! Locality/routing decisions stay in resolver/dispatch layers; descriptor and
//! catalog projection stay outside this module.

pub mod ability_management;
pub mod browser;
pub mod file_edit;
pub mod file_transfer;
pub mod files;
pub mod http;
pub mod process;
pub mod session;
pub mod shell;
pub mod terminal;
