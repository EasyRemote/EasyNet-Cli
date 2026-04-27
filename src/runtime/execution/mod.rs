// EasyNet CLI — Execution Sub-Services
// =====================================
//
// File: src/runtime/execution/mod.rs
// Description: The v10.2 sub-service partition of the Execution
//              layer. Each child module owns one slice of runtime
//              state (session / permission / discuss / schedule /
//              loop) and is forbidden — by CI grep in
//              `scripts/check-subservice-isolation.sh` — from
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
// Kernel (`runtime::kernel`), which holds one handle per sub-
// service and brokers every cross-module call. This is the same
// shape a Unix kernel enforces between subsystems, which is why
// the plan calls this a "kernel-like runtime boundary".
//
// v1 state
// --------
// Every sub-service here is a skeleton. The feature PR for each
// feature (PR-ATTACH / PR-PERM / PR-DISCUSS / PR-SCHED / PR-LOOP)
// fills in its body and its Kernel handle. Shipping the empty
// skeleton keeps every downstream PR additive — no file renames,
// no import-path breakage.
//
// Sub-service layout
// ------------------
//   session/        → one session per agent run
//   permission/     → approval broker + pending queue
//   discuss/        → multi-agent chat room registry
//   schedule/       → cron store + tick runner
//   loop_instance/  → EAL loop wrappers + status store
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod discuss;
pub mod loop_instance;
pub mod mcp_client;
pub mod permission;
pub mod pty;
pub mod schedule;
pub mod session;
