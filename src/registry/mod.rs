// EasyNet CLI — Identity & Registry Layer
// =======================================
//
// File: src/registry/mod.rs
// Description: Typed identity primitives and the local agent
//              registry they key.
//
// Contents
// --------
//
// - `agents`      — the local agent registry. Maps `AgentId`-like
//                   keys to `AgentEntry` records describing each
//                   registered CLI agent (claude, codex, …). Lives
//                   in `~/.easynet/agents.json`; persisted via
//                   `crate::persistence::config::atomic_write` so
//                   the race-safe primitive is not re-implemented.
// - `a2a_labels`  — A2A discovery codec. Projects the local
//                   `AgentRegistry` into the `a2a.agents_json`
//                   node-label the Hub advertises to peers.
//
// Identity primitives (`AgentId`, `AbilityName`, `NodeId`) live in
// `crate::core::agent_id` — the zero-dependency core layer. Registry
// depends on core, not the other way around. Callers should import
// identity types directly from `core`:
//
//     use crate::core::agent_id::{AgentId, NodeId};
//     use crate::registry::agents;
//
// Why `registry` is a top-level module
// ------------------------------------
// `agents.rs` previously lived in `shared/` alongside network
// plumbing. That meant "who are my registered agents?" and "where
// is the bridge pool?" sat in the same namespace — the
// dumping-ground shape made registry and transport look like peers
// when they are orthogonal concerns. Hoisting registry into its
// own module lets use-sites say what they actually need instead of
// reaching into a catch-all `shared` namespace.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub(crate) mod a2a_labels;
// Public so integration tests + future external embedders can
// construct AgentRegistry / AgentEntry. Field visibility on the
// types themselves stays pub(crate) — external callers go through
// the typed builders.
pub mod agents;
