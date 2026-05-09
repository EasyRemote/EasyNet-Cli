#!/usr/bin/env bash
# dev-install-local.sh — build the current EasyNet-Cli checkout and
# overwrite the system-installed binaries in place. Same shape as
# `install.sh` lays down — `easynet` + `easynet-daemon` in
# /usr/local/bin, `libaxon_dendrite_bridge.{dylib|so}` in
# ~/.easynet/dendrite-bridge/native — but pulls bytes from `cargo build`
# instead of the production tarball server.
#
# Why this exists
# ---------------
# Iterating on the CLI shape (help text, subcommand layout, new
# commands) needs the system `easynet` to be the freshly compiled one
# so `easynet --help` in any shell reflects the change. Manually doing
# `cargo build && sudo install` works but loses the dendrite bridge
# overwrite + the `axon-runtime` cleanup that production install.sh
# does, so the system can drift between dev runs.
#
# This script is the dev-loop counterpart to `install.sh`. It does the
# minimum to put your local commit in front of you:
#   1. cargo build (release by default; --debug flag for faster turnaround)
#   2. cargo build the dendrite bridge .dylib/.so
#   3. sudo install the three artefacts to the same paths install.sh uses
#   4. remove any stale `axon-runtime` binary that previous installs left
#   5. print version so you can confirm the new bytes are live
#
# Out of scope (intentional)
# --------------------------
# - cross-compile (we install for the host; cross goes through docker)
# - signing / notarisation
# - shell-rc rewrites (install.sh handles EASYNET_DENDRITE_BRIDGE_LIB
#   on first install; this script assumes you have a working install
#   already and just want to overwrite bytes)
# - tarball production (use build-release-tarball.sh for that)
#
# Usage:
#   scripts/dev-install-local.sh              # release build, then install
#   scripts/dev-install-local.sh --debug      # debug build, faster iter
#   scripts/dev-install-local.sh --no-install # build only, print path
#
# Author: Silan.Hu <silan.hu@u.nus.edu>
set -euo pipefail

# Resolve repo root from the script's own location so the caller can
# invoke this from anywhere.
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cli_root="$(cd "$script_dir/.." && pwd)"

# The dendrite bridge lives in a sibling repo (EasyNet-Axon).
# `EASYNET_BRIDGE_CRATE` overrides the path for non-standard checkouts;
# default matches the canonical sibling layout.
bridge_crate="${EASYNET_BRIDGE_CRATE:-$cli_root/../EasyNet-Axon/core/runtime-rs/dendrite-bridge}"

# Defaults — overridable via flags.
build_profile="release"
do_install=1

while [ $# -gt 0 ]; do
    case "$1" in
        --debug)       build_profile="debug" ;;
        --release)     build_profile="release" ;;
        --no-install)  do_install=0 ;;
        -h|--help)
            sed -n '2,40p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "dev-install-local.sh: unknown arg: $1" >&2
            exit 2
            ;;
    esac
    shift
done

# Platform / paths — mirror install.sh exactly so the dev install lands
# in the same place a production install would.
case "$(uname -s)" in
    Darwin) lib_ext="dylib" ;;
    Linux)  lib_ext="so" ;;
    *)
        echo "dev-install-local.sh: unsupported OS: $(uname -s)" >&2
        exit 1
        ;;
esac

install_dir="/usr/local/bin"
# Resolve real home even if the user runs this under sudo (matches
# install.sh:43-50). The dendrite bridge .dylib goes under the invoking
# user's home, not /root's, so the runtime can dlopen it.
if [ -n "${SUDO_USER:-}" ] && [ "${SUDO_USER}" != "root" ]; then
    real_home=$(getent passwd "$SUDO_USER" 2>/dev/null | cut -d: -f6 || true)
    [ -z "$real_home" ] && real_home=$(eval echo "~$SUDO_USER")
else
    real_home="$HOME"
fi
native_dir="$real_home/.easynet/dendrite-bridge/native"

cargo_args_cli=(--bin easynet --bin easynet-daemon --features axon-pb)
cargo_args_bridge=(--lib)
if [ "$build_profile" = "release" ]; then
    cargo_args_cli+=(--release)
    cargo_args_bridge+=(--release)
fi

echo "==> [1/3] cargo build easynet + easynet-daemon ($build_profile)"
(
    cd "$cli_root"
    cargo build "${cargo_args_cli[@]}"
)

if [ -d "$bridge_crate" ]; then
    echo "==> [2/3] cargo build libaxon_dendrite_bridge.${lib_ext} ($build_profile)"
    (
        cd "$bridge_crate"
        cargo build "${cargo_args_bridge[@]}"
    )
else
    # The bridge crate is in a sibling repo; if it isn't checked out,
    # warn but don't fail — the CLI binary alone is still useful for
    # iterating on `--help` / arg parsing.
    echo "==> [2/3] skipping dendrite bridge (sibling crate not found at $bridge_crate)"
fi

cli_bin="$cli_root/target/$build_profile/easynet"
daemon_bin="$cli_root/target/$build_profile/easynet-daemon"
bridge_lib="$bridge_crate/target/$build_profile/libaxon_dendrite_bridge.${lib_ext}"

for path in "$cli_bin" "$daemon_bin"; do
    if [ ! -f "$path" ]; then
        echo "dev-install-local.sh: build artefact missing: $path" >&2
        exit 1
    fi
done

if [ "$do_install" -eq 0 ]; then
    echo ""
    echo "  ✓ Build complete. Artefacts:"
    echo "      $cli_bin"
    echo "      $daemon_bin"
    [ -f "$bridge_lib" ] && echo "      $bridge_lib"
    echo ""
    echo "  Run directly without installing:"
    echo "      $cli_bin --help"
    exit 0
fi

# Install needs root for /usr/local/bin. Rather than re-exec the whole
# script under sudo (which would re-run cargo as root and pollute the
# target/ tree with root-owned files), only the install steps run via
# `sudo` — cargo build already finished above as the real user. Pre-
# authenticate once with `sudo -v` so the per-command sudo calls below
# don't each prompt.
if [ "$(id -u)" -ne 0 ]; then
    echo "==> [3/3] installing — sudo required for $install_dir"
    sudo -v
    SUDO=sudo
else
    SUDO=
fi

echo "    installing to $install_dir + $native_dir"

$SUDO install -m 755 "$cli_bin"    "$install_dir/easynet"
$SUDO install -m 755 "$daemon_bin" "$install_dir/easynet-daemon"

if [ -f "$bridge_lib" ]; then
    # native_dir lives under the real user's $HOME, not root's — so
    # mkdir + install run *without* sudo. The runtime dlopens the .dylib
    # as the user, not as root.
    mkdir -p "$native_dir"
    install -m 644 "$bridge_lib" "$native_dir/libaxon_dendrite_bridge.${lib_ext}"
fi

# Mirror install.sh's stale-binary cleanup (lines 264-276): if a
# previous install left `axon-runtime` shadowing $install_dir on
# $PATH, the daemon must not spawn it. Only remove shadowing copies.
for bin in axon-runtime; do
    for dir in /usr/local/bin /opt/homebrew/bin /usr/bin; do
        [ "$dir" = "$install_dir" ] && continue
        candidate="$dir/$bin"
        if [ -f "$candidate" ]; then
            echo "  removing stale $candidate (shadows $install_dir)"
            $SUDO rm -f "$candidate"
        fi
    done
done

echo ""
echo "  ✓ Local build installed."
echo ""
"$install_dir/easynet" --version
echo ""
echo "  Try: easynet --help"
