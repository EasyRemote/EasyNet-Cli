//! EasyNet browser plugin crate
//! ===========================
//!
//! File: plugins/browser/src/lib.rs
//! Description: Public provider export for the browser plugin package.
//!
//! Protocol Responsibility:
//! - None. Axon owns Invocation, receipts, admission, and InvokeBidi.
//!
//! Implementation Approach:
//! - Re-export the package provider linked into `easynet-daemon`.
//!
//! Usage Contract:
//! - Callers load the package and invoke governed `browser.*` abilities.
//!
//! Architectural Position:
//! - Native-static EasyNet plugin package export.

pub use easynet_cli::daemon::plugins::browser::provider;
