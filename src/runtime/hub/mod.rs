// EasyNet CLI — Hub module
// ========================
//
// File: src/runtime/hub/mod.rs
// Description: in-daemon Hub responsibilities. The daemon owns the local
//              Hub-side runtime surfaces; product backends wrap these daemon
//              abilities instead of re-implementing a second Hub runtime.
//
// Conformance: RFC-006-B v0.6 §3 (Hub: the only HTTP boundary),
//              INV-1 (Adapter Purity).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod pages_listener;
pub mod pages_serve_ability;
