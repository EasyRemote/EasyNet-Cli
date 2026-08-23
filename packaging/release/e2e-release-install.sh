#!/usr/bin/env bash
# e2e-release-install.sh — sandbox-mode replay of `packaging/release/install.sh` against
# a Phase-A tarball.
#
# What this proves
# ----------------
# The release tarball + the installer logic together produce the
# binary layout that `easynet runtime start` / `easynet device join` /
# `easynet-daemon` expect at runtime. Specifically:
#
#   * `easynet`                  is on PATH
#   * `easynet-daemon`           is on PATH
#   * `easynet-keyring`          is on PATH
#   * `libaxon_dendrite_bridge`  is at $HOME/.easynet/dendrite-bridge/native/
#   * `easynet_cli.h`            is installed under the sandbox include dir
#   * `easynet_cli.exports.v7`   is installed under the sandbox include dir
#   * `easynet_cli.exports.v8`   is installed under the sandbox include dir
#   * `ffi-abi-v7.md` and `ffi-abi-v8.md` are installed under the sandbox doc dir
#   * `EASYNET_DENDRITE_BRIDGE_LIB` env var points at that library
#   * `axon-runtime`             is NOT installed anywhere
#
# The third assertion is the load-bearing one for the production /
# docker-e2e divergence: docker images bundle axon-runtime + override
# tooling to find it; production tarballs do not. Any code path that
# *requires* axon-runtime to be a separate process will pass docker
# e2e and fail this harness.
#
# Why we don't `curl … | sudo sh`
# -------------------------------
# Two reasons:
#
#   1. CI / dev rigs should not need root. We replicate the install
#      effects against a sandbox prefix instead of /usr/local/bin.
#   2. packaging/release/install.sh fetches the tarball from
#      https://easynet.run/download — that's a network dep we don't
#      want in the inner test loop. We supply the local tarball
#      directly via `--tarball`.
#
# The "sandbox install" produces the same on-disk shape `packaging/release/install.sh`
# would; the contract being tested is "the tarball + the install
# rules" not "the curl → sudo sh ergonomics". Those are tested in
# `EasyNet/scripts/dev-host-e2e.sh` where it matters.
#
# Usage
# -----
#   packaging/release/e2e-release-install.sh \
#     [--tarball <path>] \
#     [--prefix <path>] \
#     [--keep-prefix]
#
# Defaults: tarball = newest `target/release-tarball/easynet-*.tar.gz`,
# prefix = a fresh /tmp/easynet-release-prefix-XXXXXX (auto-cleaned
# on exit unless --keep-prefix). The selected prefix is exported as
# stdout's last-line `prefix=<path>` so a caller (Phase C harness)
# can reuse it without re-deriving the location.
#
# Author: 海峰 <silan.hu@u.nus.edu>
# Copyright (c) 2026 EasyNet. All rights reserved.

set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
cli_root="$(cd "$script_dir/../.." && pwd)"

tarball=""
prefix=""
keep_prefix=0

while [ $# -gt 0 ]; do
    case "$1" in
        --tarball)    tarball="$2"; shift 2 ;;
        --prefix)     prefix="$2"; shift 2 ;;
        --keep-prefix) keep_prefix=1; shift ;;
        --help|-h)
            sed -n '2,/^# Author:/p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "e2e-release-install.sh: unknown flag $1" >&2
            exit 2
            ;;
    esac
done

# Default tarball: the freshest one Phase A produced.
if [ -z "$tarball" ]; then
    tarball="$(ls -t "$cli_root/target/release-tarball"/easynet-*.tar.gz 2>/dev/null | head -1 || true)"
    if [ -z "$tarball" ]; then
        echo "e2e-release-install.sh: no tarball passed and none in $cli_root/target/release-tarball/" >&2
        echo "  Run packaging/release/build-release-tarball.sh first." >&2
        exit 1
    fi
fi
if [ ! -f "$tarball" ]; then
    echo "e2e-release-install.sh: tarball not found: $tarball" >&2
    exit 1
fi

# Default sandbox prefix.
if [ -z "$prefix" ]; then
    prefix="$(mktemp -d /tmp/easynet-release-prefix-XXXXXX)"
fi
mkdir -p "$prefix"

cleanup() {
    if [ "$keep_prefix" = 1 ]; then
        echo "[keep-prefix] $prefix"
    else
        rm -rf "$prefix"
    fi
}
trap cleanup EXIT

# Detect platform with the same matrix packaging/release/install.sh::detect_platform
# uses, so the LIB_EXT / native subdir match what the real installer
# would have written.
case "$(uname -s)" in
    Linux)  lib_ext="so" ;;
    Darwin) lib_ext="dylib" ;;
    *) echo "e2e-release-install.sh: unsupported OS" >&2; exit 1 ;;
esac

echo "==> sandbox prefix: $prefix"
echo "==> tarball:        $tarball"

# Mirror packaging/release/install.sh layout under the sandbox prefix.
install_dir="$prefix/usr/local/bin"
include_dir="$prefix/usr/local/include/easynet"
doc_dir="$prefix/usr/local/share/doc/easynet"
easynet_home="$prefix/home/.easynet"
native_dir="$easynet_home/dendrite-bridge/native"
mkdir -p "$install_dir" "$include_dir" "$doc_dir" "$native_dir"

# Step 1: extract.
extract_dir="$prefix/extract"
mkdir -p "$extract_dir"
tar -xzf "$tarball" -C "$extract_dir"

# Step 2: required binaries. Mirror packaging/release/install.sh's "treat the daemon
# as a required artefact: if the tarball is missing it, the release
# is malformed and the installer should fail loudly."
for required in \
    easynet \
    easynet-daemon \
    easynet-keyring \
    "libaxon_dendrite_bridge.${lib_ext}" \
    include/easynet_cli.h \
    include/easynet_cli.exports.v7 \
    include/easynet_cli.exports.v8 \
    docs/spec/ffi-abi-v7.md \
    docs/spec/ffi-abi-v8.md
do
    if [ ! -f "$extract_dir/$required" ]; then
        echo "[FAIL] tarball missing required artefact: $required" >&2
        exit 1
    fi
done

# Step 3: production-shape contract — axon-runtime must NOT be in
# the tarball. This is the assertion that catches regressions where
# someone re-bundles axon-runtime into a release.
if [ -f "$extract_dir/axon-runtime" ]; then
    echo "[FAIL] tarball illegally ships axon-runtime; release is hub-only-bound" >&2
    echo "       offending file: $extract_dir/axon-runtime" >&2
    exit 1
fi

# Step 4: install (mirror packaging/release/install.sh::download_and_install).
mv "$extract_dir/easynet"        "$install_dir/easynet"
mv "$extract_dir/easynet-daemon" "$install_dir/easynet-daemon"
mv "$extract_dir/easynet-keyring" "$install_dir/easynet-keyring"
chmod +x "$install_dir/easynet" "$install_dir/easynet-daemon" "$install_dir/easynet-keyring"
mv "$extract_dir/libaxon_dendrite_bridge.${lib_ext}" "$native_dir/"
mv "$extract_dir/include/easynet_cli.h" "$include_dir/easynet_cli.h"
mv "$extract_dir/include/easynet_cli.exports.v7" "$include_dir/easynet_cli.exports.v7"
mv "$extract_dir/include/easynet_cli.exports.v8" "$include_dir/easynet_cli.exports.v8"
mv "$extract_dir/docs/spec/ffi-abi-v7.md" "$doc_dir/ffi-abi-v7.md"
mv "$extract_dir/docs/spec/ffi-abi-v8.md" "$doc_dir/ffi-abi-v8.md"

# Step 5: env stamping (mirror packaging/release/install.sh::setup_env). We don't
# write into a real shell profile — the harness's caller picks up
# the env via the `env=<path>` line we emit at the bottom.
env_file="$prefix/easynet-env.sh"
cat > "$env_file" <<EOF
# Sourced by e2e-release-flow.sh. Mirrors what packaging/release/install.sh appends
# to a real user's shell profile.
export EASYNET_DENDRITE_BRIDGE_LIB="$native_dir/libaxon_dendrite_bridge.${lib_ext}"
export PATH="$install_dir:\$PATH"
# Sandbox HOME so each Phase-C run gets its own credentials.json /
# local-agents.json / runtime.json without touching the real user's
# tree.
export HOME="$prefix/home"
EOF

# Step 6: post-install assertions.
fail=0
for assert in \
    "$install_dir/easynet:executable" \
    "$install_dir/easynet-daemon:executable" \
    "$install_dir/easynet-keyring:executable" \
    "$native_dir/libaxon_dendrite_bridge.${lib_ext}:exists" \
    "$include_dir/easynet_cli.h:exists" \
    "$include_dir/easynet_cli.exports.v7:exists" \
    "$include_dir/easynet_cli.exports.v8:exists" \
    "$doc_dir/ffi-abi-v7.md:exists" \
    "$doc_dir/ffi-abi-v8.md:exists"
do
    path="${assert%:*}"
    kind="${assert##*:}"
    if [ ! -e "$path" ]; then
        echo "[FAIL] post-install: $path missing" >&2
        fail=1
        continue
    fi
    case "$kind" in
        executable)
            if [ ! -x "$path" ]; then
                echo "[FAIL] post-install: $path not executable" >&2
                fail=1
            fi
            ;;
    esac
done

# axon-runtime must not be anywhere under the sandbox prefix
# post-install — including the install dir, the dendrite-bridge dir,
# or any subdir the dependency-graph might have leaked it into.
stray="$(find "$prefix" -name "axon-runtime" -type f 2>/dev/null | head -1 || true)"
if [ -n "$stray" ]; then
    echo "[FAIL] axon-runtime must not be installed; found at $stray" >&2
    fail=1
fi

if [ "$fail" != 0 ]; then
    exit 1
fi

if ! grep -q '#define RUNTIME_ABI_VERSION 7u' "$include_dir/easynet_cli.h"; then
    echo "[FAIL] installed easynet_cli.h does not declare ABI version 5" >&2
    fail=1
fi

if [ "$(wc -l < "$include_dir/easynet_cli.exports.v7" | tr -d ' ')" != "56" ] ||
   ! LC_ALL=C sort -c "$include_dir/easynet_cli.exports.v7" 2>/dev/null; then
    echo "[FAIL] installed easynet_cli.exports.v7 is not the exact sorted 56-symbol contract" >&2
    fail=1
fi

if [ "$(wc -l < "$include_dir/easynet_cli.exports.v8" | tr -d ' ')" != "57" ] ||
   ! LC_ALL=C sort -c "$include_dir/easynet_cli.exports.v8" 2>/dev/null; then
    echo "[FAIL] installed easynet_cli.exports.v8 is not the exact sorted 57-symbol contract" >&2
    fail=1
fi

if ! comm -23 "$include_dir/easynet_cli.exports.v7" "$include_dir/easynet_cli.exports.v8" | sed '/^$/d' >"$prefix/v8-missing-v7"; then
    true
fi
if [ -s "$prefix/v8-missing-v7" ]; then
    echo "[FAIL] installed easynet_cli.exports.v8 does not include every v7 symbol" >&2
    cat "$prefix/v8-missing-v7" >&2
    fail=1
fi

if ! comm -13 "$include_dir/easynet_cli.exports.v7" "$include_dir/easynet_cli.exports.v8" >"$prefix/v8-added"; then
    true
fi
if [ "$(cat "$prefix/v8-added")" != "runtime_invocation_stream_open_v8" ]; then
    echo "[FAIL] installed easynet_cli.exports.v8 must add only runtime_invocation_stream_open_v8" >&2
    cat "$prefix/v8-added" >&2
    fail=1
fi

if ! grep -q 'include/easynet_cli.h' "$doc_dir/ffi-abi-v7.md"; then
    echo "[FAIL] installed ffi-abi-v7.md does not reference the C header contract" >&2
    fail=1
fi
if ! grep -q 'runtime_invocation_stream_open_v8' "$doc_dir/ffi-abi-v8.md"; then
    echo "[FAIL] installed ffi-abi-v8.md does not define the raw stream extension" >&2
    fail=1
fi

# Smoke: spawn `easynet --version` against the sandbox env to prove
# the binary at least loads its own libs.
(
    # shellcheck disable=SC1090
    . "$env_file"
    if ! version_out="$("$install_dir/easynet" --version 2>&1)"; then
        echo "[FAIL] easynet --version exited non-zero" >&2
        echo "$version_out" >&2
        exit 1
    fi
    echo "==> easynet --version: $version_out"
)

# Daemon binary smoke: confirm the binary runs at all. Daemon refuses
# subcommands by design (it's an IPC child spawned by `easynet
# runtime start`), so `--help` exits non-zero with a polite redirect
# message — not a launch failure. We check that:
#
#   * the binary exits without a linker / load error (e.g. a missing
#     dylib reference would exit before printing anything), and
#   * the rejection message is the documented one.
#
# A real load failure shows up as exit > 0 with NO output (loader
# rejects the binary before main runs).
(
    # shellcheck disable=SC1090
    . "$env_file"
    daemon_smoke="$("$install_dir/easynet-daemon" --help 2>&1 || true)"
    case "$daemon_smoke" in
        *"this binary takes no command arguments"*)
            : # expected polite-rejection path; binary loaded fine.
            ;;
        *)
            echo "[FAIL] easynet-daemon smoke produced unexpected output:" >&2
            printf '%s\n' "$daemon_smoke" >&2
            exit 1
            ;;
    esac
)

echo
echo "[OK] release-shape install verified"
echo "  install_dir: $install_dir"
echo "  native_dir:  $native_dir"
echo "  include_dir: $include_dir"
echo "  doc_dir:     $doc_dir"
echo "  env file:    $env_file"
echo "  binaries:    easynet, easynet-daemon, easynet-keyring"
echo "  library:     libaxon_dendrite_bridge.${lib_ext}"
echo "  c abi:       easynet_cli.h + easynet_cli.exports.v7 + easynet_cli.exports.v8 (generic ABI v7 with v8 raw-stream extension)"
echo "  forbidden:   axon-runtime (absent ✓)"
echo
# Last-line contract for Phase C consumers: env=<path> + prefix=<path>
echo "env=$env_file"
echo "prefix=$prefix"
