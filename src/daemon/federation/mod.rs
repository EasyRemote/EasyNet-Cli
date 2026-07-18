// EasyNet CLI - daemon federation domain
// =======================================
//
// File: src/daemon/federation/mod.rs
// Description: Daemon-owned federation policy, wire vocabulary, and runtime
//              adapters carried through Axon's generic Invocation transport.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod advertise;
pub mod client;
#[cfg(feature = "axon-pb")]
pub mod directory;
pub mod directory_reader;
pub mod peers;
pub mod read_model;
pub mod resolver;
pub(crate) mod resolver_contract;
pub(crate) mod wire_contract;
