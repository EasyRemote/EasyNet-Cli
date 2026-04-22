// EasyNet CLI — Facade Layer
// ==========================
//
// File: src/facade/mod.rs
// Description: User-facing entry points. This layer exposes the CLI
//              subcommand tree and the outbound MCP tool provider.
//              Everything here should be thin — the substantive
//              logic lives in `runtime/`, `eal/`, `registry/`,
//              `persistence/`, `core/`.
//
// Layering rule:
//   facade → eal & runtime → registry & persistence → core & support
//
//   facade depends downward only. Modules below must never import
//   from facade; doing so would create a cycle and reintroduce the
//   dumping-ground shape the recent refactor just unwound.
//
// Why `cli` and `mcp` live here:
//   Both are surfaces a user (or another agent) hits from outside
//   the process. Treating them as peers under `facade/` makes it
//   obvious when a new surface (e.g. a WS control plane, a future
//   gRPC endpoint) should sit alongside them instead of
//   re-implementing its own copy of the business logic below.
//
// Empty placeholder modules (`daemon`, `publish`, `services`,
// `transport`, `pairing`) were deliberately removed from this
// tree: violating the consumer-driven rule by keeping them
// present would misrepresent the state of work to a reviewer. A
// future PR that actually implements any of those surfaces
// creates the module at that time.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod cli;
pub mod mcp;
