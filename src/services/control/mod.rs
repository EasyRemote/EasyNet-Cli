// EasyNet CLI — Local Control Plane
// ==================================
//
// File: src/services/control/mod.rs
// Description: The local IPC surface a Client FFI library uses to
//              reach this daemon. v1 transport is a Unix Domain
//              Socket on Linux/macOS and a Named Pipe on Windows;
//              auth is provided by filesystem permissions (UDS mode
//              0600 / Named-Pipe ACL pinned to the current user SID)
//              — no bearer tokens.
//
// Why Named-Pipe + UDS, not WebSocket
// -----------------------------------
// Plan v10.1 argued the decision in full; the short version: the
// Client is always same-host, so TCP+WS is pure overhead. UDS/Pipe
// auth piggybacks on the OS's process-user model, which is both
// stronger (another user physically cannot open the socket) and
// cheaper (no token issuance / rotation / expiry). The one thing
// WS gives you — browser DevTools inspection — is useless when the
// consumer is a native Client process loading a cdylib.
//
// Module layout
// -------------
//   transport.rs     — cross-platform listener abstraction
//                      (UDS + Named Pipe) behind a single trait.
//   discovery.rs     — reads/writes ~/.easynet/control.json so the
//                      lib can find the socket without guessing.
//   ability_proxy.rs — frame-decode ↔ KernelApi adapter: one
//                      function per wire verb (invoke / subscribe /
//                      cancel). The only surface that speaks proto
//                      message JSON → domain-object call.
//   server.rs        — accept-loop + per-connection spawn; ties
//                      transport + ability_proxy together.
//   frames.rs        — length-prefixed JSON frame codec types.
//
// v1 status
// ---------
// All submodules here ship as *skeletons*. A skeleton in this
// layer is:
//   (a) a public struct with a `new()` constructor,
//   (b) method signatures that compile against the trait boundaries
//       above, and
//   (c) bodies that `bail!` with a "not yet wired" message.
//
// This shape means a follow-up PR can (i) write a single method
// body, (ii) run `cargo check --bin easynet-daemon`, and (iii)
// ship — without touching the public API. Feature PRs use the same
// pattern against their respective Execution sub-services.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod ability_proxy;
pub mod discovery;
pub mod frames;
/// Step 3 of the cross-repo plan: separate UDS responder for
/// runtime-routed Invokes that arrived at axon-runtime for an
/// ability the daemon registered via `runtime.register_local_tool`.
/// Distinct from `server.rs` (length-delimited JSON IPC for CLI
/// subcommands + local stdio MCP) — the runtime side speaks
/// newline-delimited single-line JSON instead.
pub mod runtime_dispatch;
pub mod server;
pub mod transport;
