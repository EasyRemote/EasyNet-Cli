// EasyNet CLI - daemon federation domain
// =======================================
//
// File: src/daemon/federation/mod.rs
// Description: Daemon-owned federation runtime adapters. Axon owns the
//              cross-language protocol contracts; this module owns how
//              easynet-daemon tracks, reads, dials, and supervises peer hubs.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

#[cfg(feature = "axon-pb")]
pub mod client;
#[cfg(feature = "axon-pb")]
pub mod directory;
pub mod directory_reader;
pub mod gateway;
pub mod gateway_api;
pub mod peers;
pub mod read_model;
