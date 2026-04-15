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
// - `agent_id`    — typed identity primitives (`AgentId`,
//                   `AbilityName`, `NodeId`). The L2 identity layer
//                   specified in `docs/AGENT_IDENTITY.md`. These
//                   are pure data types with validation; they do
//                   not touch the filesystem or the network.
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
// Why `registry` is a top-level module
// ------------------------------------
// `agents.rs` and `agent_id.rs` previously lived in `shared/`
// alongside network plumbing. That meant "who are my registered
// agents?" and "where is the bridge pool?" sat in the same
// namespace — the dumping-ground shape made identity and transport
// look like peers when they are orthogonal concerns. Hoisting
// identity into `registry/` lets its use-sites say what they
// actually need:
//
//     use crate::registry::agents;
//     use crate::registry::agent_id::{AgentId, NodeId};
//
// instead of the previously misleading `crate::shared::*`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub(crate) mod a2a_labels;
pub(crate) mod agent_id;
pub(crate) mod agents;
