// EasyNet CLI — Services Layer
// =============================
//
// File: src/services/mod.rs
// Description: Long-running, non-CLI surfaces hosted by the daemon
//              process. Today that means one thing — the local
//              `control` plane that Client FFI libraries dial into —
//              but the layer exists as a place for future sibling
//              services (IPC-over-vsock for a future hypervisor mode,
//              a planner service, etc.) to sit without polluting
//              `facade/` or `runtime/`.
//
// Layering rule
// -------------
// `services/` sits at the same layer as `facade/` and imports from
// the runtime layer only through the two hard trait boundaries:
//
//   * KernelApi  (runtime/kernel_api.rs)  — for invoking abilities,
//                                          listing sessions, etc.
//   * GatewayApi (runtime/gateway_api.rs) — NOT used here; only the
//                                          runtime itself speaks to
//                                          the gateway.
//
// `scripts/check-kernel-boundary.sh` greps `src/services/` for any
// `use crate::runtime::...` import that is not `kernel_api`,
// `invocation`, `domain`, or `receipt_subscriber`. Anything else is
// a boundary violation and fails CI.
//
// Why a separate top-level layer instead of a sibling under facade/
// -----------------------------------------------------------------
// `facade/cli` and `facade/mcp` are user-or-agent surfaces hit from
// *outside* the daemon process: the CLI user's terminal, a remote
// MCP client. `services/control` is an *intra-machine* surface
// hit by a Client FFI library loaded into another process on the
// same host. The trust model, transport, and lifetime are all
// different enough that keeping them in peer namespaces prevents
// accidental conflation (e.g. a CLI subcommand reaching into the
// IPC server's state, or vice versa).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod control;
