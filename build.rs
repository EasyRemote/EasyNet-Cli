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
    // Re-run build script when any .proto file changes. These
    // directives apply whether or not any feature is active, so that
    // re-enabling either feature after an edit picks up the change
    // without a `cargo clean`.
    println!("cargo:rerun-if-changed=schemas/common.proto");
    println!("cargo:rerun-if-changed=schemas/control_plane.proto");
    println!("cargo:rerun-if-changed=build.rs");

    // RFC-003 PR-1: when the `axon-pb` feature is enabled, also
    // monitor every axon `.proto` so re-builds pick up upstream
    // changes. The directives are emitted unconditionally because
    // a developer who flips the feature later should not need a
    // `cargo clean` either.
    rerun_on_axon_proto_dir();

    #[cfg(feature = "proto-gen")]
    compile_proto();

    #[cfg(feature = "axon-pb")]
    compile_axon_proto();
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

/// Path to axon's canonical `.proto` set, resolved relative to the
/// EasyNet-Cli crate root. Mirrors the path used by
/// `EasyNet-Federation-MVP/common/build.rs` so both crates compile
/// against the same byte-identical sources. RFC-003 spec §0 forbids
/// modifying axon repo; this build script reads only.
const AXON_PROTO_ROOT: &str = "../EasyNet-Axon/core/runtime-rs/client-sdk/proto";

fn rerun_on_axon_proto_dir() {
    // Even when the axon-pb feature is off we still want to know
    // when the proto root appears or moves, so flipping the feature
    // never silently builds against stale generated code.
    println!("cargo:rerun-if-changed={AXON_PROTO_ROOT}");
}

#[cfg(feature = "axon-pb")]
fn compile_axon_proto() {
    use std::fs;
    use std::path::PathBuf;

    let proto_root = PathBuf::from(AXON_PROTO_ROOT);
    let proto_dir = proto_root.join("axon/v1");
    if !proto_dir.is_dir() {
        panic!(
            "EasyNet-Cli: feature `axon-pb` requires the axon proto set at {} \
             (typically the EasyNet-Axon repo checked out as a sibling of EasyNet-Cli/)",
            proto_dir.display()
        );
    }

    let mut protos: Vec<PathBuf> = fs::read_dir(&proto_dir)
        .expect("read axon proto dir")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension().and_then(|ext| ext.to_str()) == Some("proto")).then_some(path)
        })
        .collect();
    protos.sort();

    for path in &protos {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&protos, &[proto_root.as_path()])
        .expect("tonic_build compile failed for axon protos");
}
