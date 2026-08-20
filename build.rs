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
// src/daemon/control/frames.rs); a premature flip to
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
    println!("cargo:rerun-if-changed=plugins/desktop-menubar/plugin.toml");
    println!("cargo:rerun-if-changed=plugins/desktop-menubar/scripts/build-macos.sh");
    println!(
        "cargo:rerun-if-changed=plugins/desktop-menubar/companion/macos/EasyNetMenuBar/Info.plist"
    );
    println!(
        "cargo:rerun-if-changed=plugins/desktop-menubar/companion/macos/EasyNetMenuBar/Sources/EasyNetMenuBar/main.swift"
    );
    println!(
        "cargo:rerun-if-changed=plugins/desktop-menubar/companion/macos/EasyNetMenuBar/Resources"
    );

    materialize_desktop_menubar_package();

    #[cfg(feature = "proto-gen")]
    compile_proto();
}

fn materialize_desktop_menubar_package() {
    let Ok(target_os) = std::env::var("CARGO_CFG_TARGET_OS") else {
        return;
    };
    if target_os != "macos" {
        return;
    }

    let manifest_dir =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let package_root = out_dir
        .join("builtin-plugins")
        .join("easynet.desktop.menubar");
    let script = manifest_dir
        .join("plugins")
        .join("desktop-menubar")
        .join("scripts")
        .join("build-macos.sh");

    std::fs::create_dir_all(&package_root).expect("create desktop menubar package root");
    std::fs::copy(
        manifest_dir
            .join("plugins")
            .join("desktop-menubar")
            .join("plugin.toml"),
        package_root.join("plugin.toml"),
    )
    .expect("copy desktop menubar plugin manifest into materialized package");

    let status = std::process::Command::new(&script)
        .env("EASYNET_DESKTOP_MENUBAR_PACKAGE_ROOT", &package_root)
        .status()
        .expect("run desktop menubar macOS build script");
    if !status.success() {
        panic!("desktop menubar macOS build script failed with {status}");
    }

    println!(
        "cargo:rustc-env=EASYNET_DESKTOP_MENUBAR_PACKAGE_ROOT={}",
        package_root.display()
    );
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
