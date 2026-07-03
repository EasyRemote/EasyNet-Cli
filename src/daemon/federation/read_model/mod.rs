// EasyNet CLI - daemon federation read models
// ===========================================
//
// File: src/daemon/federation/read_model/mod.rs
// Description: In-memory projections maintained by daemon federation
//              handlers and read by daemon discovery/catalog surfaces.
//
// These stores are not Axon protocol authority. They cache admitted
// federation facts so resolver and catalog paths can answer bounded read
// queries without re-running transport fan-out.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub(crate) mod a2a_labels;
#[cfg(feature = "axon-pb")]
pub mod ability_catalog;
pub mod advertised_agents;
pub mod hub_published_abilities;
pub(crate) mod owner_projection;
