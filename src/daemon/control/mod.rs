// EasyNet CLI — Local Control Plane
// ==================================
//
// File: src/daemon/control/mod.rs
// Description: Local daemon IPC surfaces owned by EasyNet-Cli. The
//              public control socket is boot/status only; product
//              ability calls use the daemon Invocation API instead.
//              v1 transport is a Unix Domain Socket on Linux/macOS
//              and a Named Pipe on Windows; auth is provided by
//              filesystem permissions (UDS mode 0600 / Named-Pipe
//              ACL pinned to the current user SID) — no bearer tokens.
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
//   server.rs        — control.sock accept loop for boot/status
//                      subscriptions and diagnostics.
//   frames.rs        — boot/status-only length-prefixed JSON frame
//                      codec types.
// v1 status
// ---------
// `control.sock` is no longer a product ability transport. Keep it
// narrow: boot lifecycle events, status discovery, and protocol
// diagnostics. Every product call enters through daemon Invocation;
// the daemon dispatches admitted calls directly to its embedded
// Axon `LocalRuntime`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod boot_events;
pub mod discovery;
pub mod frames;
pub mod server;
pub mod transport;
