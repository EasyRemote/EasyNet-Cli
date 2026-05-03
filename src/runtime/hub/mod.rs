// EasyNet CLI — Hub module
// ========================
//
// File: src/runtime/hub/mod.rs
// Description: in-daemon Hub responsibilities. v0 carries only
//              the Pages reference system's listener (RFC-006-B
//              v0.6). Production traffic terminates at the Go
//              backend's wildcard listener; this in-daemon
//              listener is the dev-mode existence proof and
//              the path the MVP demo uses.
//
// Conformance: RFC-006-B v0.6 §3 (Hub: the only HTTP boundary),
//              INV-1 (Adapter Purity).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod pages_listener;
pub mod pages_serve_ability;
