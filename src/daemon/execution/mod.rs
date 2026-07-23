// EasyNet CLI — Execution Sub-Services
// =====================================
//
// File: src/daemon/execution/mod.rs
// Description: Daemon-owned long-lived execution sub-services.
//              Each child module owns one slice of execution
//              state (session / permission / mission / schedule /
//              loop) and is forbidden — by CI grep in
//              `tools/scripts/check-subservice-isolation.sh` — from
//              importing sibling sub-services.
//
// Why this partition exists
// -------------------------
// v10.1's earlier architecture had a monolithic Execution layer
// where schedule, permission, and session all shared state on a
// single struct. Reviewers flagged the implicit contract surface
// (every new feature could touch any state) and the cascade risk
// (one sub-service's bug corrupted the rest). v10.2 pins the
// boundaries at the module level so the CI grep can enforce them.
//
// Communication between sub-services always goes through the
// Kernel (`daemon::boot::kernel`), which holds one handle per sub-
// service and brokers every cross-module call. The execution
// services own daemon state; the Kernel remains the chokepoint
// for cross-service orchestration and Invocation entry.
//
// State ownership
// ---------------
// These modules are intentionally stateful services, not ability
// handler bodies. Handlers receive typed handles from catalog/daemon
// construction and call into these services; the services do not
// import handler modules to discover product ability names.
//
// Sub-service layout
// ------------------
//   session/        → one session per agent run
//   permission/     → approval broker + pending queue
//   mission/        → local mission runner and mission-scoped state
//   schedule/       → cron store + tick runner
//   loop_instance/  → EAL loop wrappers + status store
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub(crate) mod child_invocation;
pub mod loop_instance;
pub mod mcp;
pub mod mission;
pub mod permission;
pub mod pty;
pub mod runtime_identity;
pub mod schedule;
pub mod session;
