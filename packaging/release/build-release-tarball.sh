#!/usr/bin/env bash
# build-release-tarball.sh — produce a tarball that 1:1 mirrors the
# production release shape that `packaging/release/install.sh` consumes.
#
# What this script ships
# ----------------------
# Runtime artefacts are flat at the tarball root; binding/doc
# artefacts keep their normal repository paths:
#
#   easynet                              — CLI binary
#   easynet-daemon                       — long-running daemon
#   easynet-keyring                      — device-signing vault helper
#   libaxon_dendrite_bridge.{dylib|so}   — dendrite SDK shared library
#   include/easynet_cli.h                — libeasynet_cli generic C ABI v7 header
#   include/easynet_cli.exports.v7       — exact 56-symbol export allowlist
#   docs/spec/ffi-abi-v7.md              — binding-facing ABI spec
#
# Critically: NO `axon-runtime`. The production installer cleans up
# any axon-runtime binary it finds (`packaging/release/install.sh` lines around 264-268
# treat it as a stale artefact). The release tarball must mirror
# that — every test that exercises `easynet runtime start` against a real
# release shape needs to confirm the device-mode flow does not depend
# on a separate axon-runtime process.
#
# Why this exists
# ---------------
# `EasyNet/scripts/docker-build-images.sh` builds the docker e2e images and
# DOES bundle axon-runtime + libaxon_dendrite_bridge into them, which
# silently masks bugs in the device-mode boot path: any code that
# tries to spawn axon-runtime succeeds inside docker e2e because the
# binary is right there. Production tarballs do not carry it. That
# divergence is exactly what hides the "easynet runtime start spawns
# axon-runtime which doesn't exist on a freshly-installed host" bug.
#
# Output
# ------
#   target/release-tarball/easynet-<arch_tag>-<os_tag>.tar.gz
#
# arch_tag / os_tag follow the same matrix `packaging/release/install.sh::detect_platform`
# uses, so the resulting tarball is byte-shape-equivalent to a real
# `easynet-<target>.tar.gz` download from `https://easynet.run/`.
#
# Author: 海峰 <silan.hu@u.nus.edu>
# Copyright (c) 2026 EasyNet. All rights reserved.

set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
cli_root="$(cd "$script_dir/../.." && pwd)"
workspace_root="$(cd "$cli_root/.." && pwd)"
axon_root="$workspace_root/EasyNet-Axon"
bridge_crate="$axon_root/core/runtime-rs/dendrite-bridge"

# Detect host platform with the SAME matrix packaging/release/install.sh::detect_platform
# uses. Crucially we keep `aarch64-unknown-linux-gnu` and
# `aarch64-apple-darwin` distinct because the dendrite bridge is a
# native shared library and links against libc / libsystem-foundation.
host_os="$(uname -s)"
case "$host_os" in
    Linux)
        os_tag="unknown-linux-gnu"
        lib_ext="so"
        rust_target_suffix="unknown-linux-gnu"
        ;;
    Darwin)
        os_tag="apple-darwin"
        lib_ext="dylib"
        rust_target_suffix="apple-darwin"
        ;;
    *)
        echo "build-release-tarball.sh: unsupported OS $host_os" >&2
        exit 1
        ;;
esac

host_arch="$(uname -m)"
case "$host_arch" in
    arm64|aarch64) arch_tag="aarch64" ;;
    x86_64|amd64)  arch_tag="x86_64" ;;
    *)
        echo "build-release-tarball.sh: unsupported arch $host_arch" >&2
        exit 1
        ;;
esac

# Rust target triple matches the packaging/release/install.sh-emitted TARGET except for
# the legacy alias arm64 → aarch64 normalization above.
rust_target="${arch_tag}-${rust_target_suffix}"
target_tag="${arch_tag}-${os_tag}"

build_profile="${EASYNET_RELEASE_PROFILE:-release}"
case "$build_profile" in
    release|debug) ;;
    *) echo "build-release-tarball.sh: invalid profile $build_profile (release|debug)" >&2; exit 1 ;;
esac

# Output path. Uses EasyNet-Cli's own target/ tree so a `cargo clean`
# wipes test artefacts alongside build artefacts.
out_dir="$cli_root/target/release-tarball"
out_file="$out_dir/easynet-${target_tag}.tar.gz"
mkdir -p "$out_dir"

stage_dir="$(mktemp -d /tmp/easynet-release-stage-XXXXXX)"
trap 'rm -rf "$stage_dir"' EXIT

cargo_args_cli=(--bin easynet --bin easynet-daemon --bin easynet-keyring)
cargo_args_bridge=(--lib)
if [ "$build_profile" = "release" ]; then
    cargo_args_cli=("${cargo_args_cli[@]}" --release)
    cargo_args_bridge=("${cargo_args_bridge[@]}" --release)
fi

echo "==> [1/3] building easynet + easynet-daemon + easynet-keyring ($rust_target, $build_profile)"
(
    cd "$cli_root"
    # We deliberately do NOT pass --target on host-native builds; the
    # default target tree is what cargo install would use, and
    # packaging/release/install.sh expects the same per-OS/arch layout. Use --target
    # only when cross-compiling (which build-release-tarball.sh does
    # not do today; cross-compile is left to docker-build-images).
    cargo build "${cargo_args_cli[@]}" >&2
)

echo "==> [2/3] building libaxon_dendrite_bridge.${lib_ext} ($build_profile)"
(
    cd "$bridge_crate"
    cargo build "${cargo_args_bridge[@]}" >&2
)

# Source paths for the three release artefacts.
cli_bin="$cli_root/target/$build_profile/easynet"
daemon_bin="$cli_root/target/$build_profile/easynet-daemon"
keyring_bin="$cli_root/target/$build_profile/easynet-keyring"
bridge_lib="$bridge_crate/target/$build_profile/libaxon_dendrite_bridge.${lib_ext}"
abi_header="$cli_root/include/easynet_cli.h"
abi_exports="$cli_root/include/easynet_cli.exports.v7"
abi_spec="$cli_root/docs/spec/ffi-abi-v7.md"

for path in "$cli_bin" "$daemon_bin" "$keyring_bin" "$bridge_lib" "$abi_header" "$abi_exports" "$abi_spec"; do
    if [ ! -f "$path" ]; then
        echo "build-release-tarball.sh: expected artefact missing: $path" >&2
        exit 1
    fi
done

# Defense-in-depth: if axon-runtime got built into the same target
# tree from a sibling cargo invocation, refuse to include it. The
# tarball MUST NOT carry it.
stray_axon="$cli_root/target/$build_profile/axon-runtime"
if [ -f "$stray_axon" ]; then
    echo "build-release-tarball.sh: refusing to ship; stray axon-runtime at $stray_axon" >&2
    echo "  This script must produce a release-shape tarball with NO axon-runtime." >&2
    echo "  Move the binary aside if you need it for a separate hub-mode tarball." >&2
    exit 1
fi

echo "==> [3/3] staging tarball at $out_file"
cp "$cli_bin"    "$stage_dir/easynet"
cp "$daemon_bin" "$stage_dir/easynet-daemon"
cp "$keyring_bin" "$stage_dir/easynet-keyring"
cp "$bridge_lib" "$stage_dir/libaxon_dendrite_bridge.${lib_ext}"
mkdir -p "$stage_dir/include" "$stage_dir/docs/spec"
cp "$abi_header" "$stage_dir/include/easynet_cli.h"
cp "$abi_exports" "$stage_dir/include/easynet_cli.exports.v7"
cp "$abi_spec" "$stage_dir/docs/spec/ffi-abi-v7.md"

# Strip symbols on release builds to match what production tarballs
# look like; debug profile keeps symbols for stack traces.
if [ "$build_profile" = "release" ] && command -v strip >/dev/null 2>&1; then
    strip "$stage_dir/easynet" "$stage_dir/easynet-daemon" "$stage_dir/easynet-keyring" 2>/dev/null || true
fi

# tar -C into the staging dir so the tarball entries are flat
# (`easynet`, not `tmp/easynet-release-stage-XXX/easynet`).
tar -czf "$out_file" -C "$stage_dir" \
    easynet \
    easynet-daemon \
    easynet-keyring \
    "libaxon_dendrite_bridge.${lib_ext}" \
    include/easynet_cli.h \
    include/easynet_cli.exports.v7 \
    docs/spec/ffi-abi-v7.md

echo
echo "[OK] release tarball ready"
echo "  path:    $out_file"
echo "  shape:   easynet, easynet-daemon, easynet-keyring, libaxon_dendrite_bridge.${lib_ext}, include/easynet_cli.h, include/easynet_cli.exports.v7, docs/spec/ffi-abi-v7.md"
echo "  axon-runtime: NOT shipped (production-shape contract)"
echo "  size:    $(wc -c < "$out_file" | awk '{printf "%.1f MiB", $1/1024/1024}')"
