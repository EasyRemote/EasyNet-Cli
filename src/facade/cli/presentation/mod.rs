// EasyNet CLI — presentation layer
// =================================
//
// File: src/facade/cli/presentation/mod.rs
// Description: CLI-only presentation primitives. Anything that
//              renders an interactive UX surface (spinner, banner,
//              live stage stream, future progress bars) lives here.
//
// Layer boundary
// --------------
// `support::output` holds the project-level output API (success /
// warn / info / detail / kv_section). It has no terminal-UI
// dependencies and is callable from any layer.
//
// This module is one layer up: CLI-only, may depend on indicatif /
// console for live rendering, and is consumed by `facade::cli::*`
// commands. Nothing outside `facade::cli::` should import from
// here.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod banner;
pub mod stage;
