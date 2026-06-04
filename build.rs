// EasyNet CLI — build script for proto compilation
// =================================================
//
// File: build.rs
// Description: Compiles `schemas/*.proto` into Rust types via
//              prost-build when the `proto-gen` feature is enabled.
//              No-op otherwise so developers without protoc
//              installed can still build the CLI.
//
// Enabled / disabled by feature flag
// ----------------------------------
// Feature `proto-gen` is off by default (see Cargo.toml `[features]`).
// When enabled:
//   cargo build --features proto-gen   → runs prost_build::compile_protos
// When disabled:
//   cargo build                         → this script exits with
//                                         a cargo:rerun-if-changed
//                                         directive and no compile
//
// The generated types land in `$OUT_DIR/easynet.v1.rs` and
// `$OUT_DIR/easynet.v1.control.rs`; a `mod proto;` in src/ pulls
// them in via `include!(concat!(env!("OUT_DIR"), "/..."))`. That
// follow-up wiring is NOT part of this commit — this build script
// exists to land the tooling, not to flip the call sites.
//
// Why feature-gated instead of always-on
// --------------------------------------
// Plan v10.5 R1 wants Client bindings to use proto as the cross-
// language source of truth. But on the Rust daemon side we are
// still writing JSON frames directly (see
// src/services/control/frames.rs); a premature flip to
// prost-generated types would double-specify the same shapes.
// Gating behind `proto-gen` lets:
//   (1) the schema files land as truth source for other-language
//       bindings immediately (protoc works on `schemas/*.proto`
//       regardless of this crate's features),
//   (2) Rust-side consumption phase in only when the next PR-DAEMON
//       follow-up commit flips frames.rs to the generated types.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

fn main() {
    // Re-run build script when any CLI-owned .proto file changes.
    // These directives apply whether or not `proto-gen` is active, so
    // re-enabling it after an edit picks up the change without a
    // `cargo clean`.
    println!("cargo:rerun-if-changed=schemas/common.proto");
    println!("cargo:rerun-if-changed=schemas/control_plane.proto");
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(feature = "proto-gen")]
    compile_proto();
}

#[cfg(feature = "proto-gen")]
fn compile_proto() {
    // Compile both proto files in one invocation so prost-build
    // resolves the `import "common.proto";` reference from
    // control_plane.proto without a separate pass.
    prost_build::Config::new()
        .compile_protos(
            &["schemas/common.proto", "schemas/control_plane.proto"],
            &["schemas"],
        )
        .expect("compile_protos: prost-build failed — is `protoc` installed and on PATH?");
}
