// EasyNet CLI - daemon ability services
// ======================================
//
// File: src/daemon/ability/mod.rs
// Description: Daemon-owned ability services that support descriptor
//              publication and operator-facing ability metadata.
//
// This module does not redefine the runtime AbilityDescriptor model in
// `runtime::ability`; it owns daemon process services that observe or
// enrich ability surfaces.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod health;
pub mod names;
