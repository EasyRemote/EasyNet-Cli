#!/usr/bin/env bash
# Build the Linux CLI runtime artifact bundle consumed by Docker E2E images.
#
# This script is intentionally host-driven: Docker image assembly receives a
# complete runtime bundle and does not compile Rust crates as a hidden fallback.
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
WORKSPACE_ROOT="$(cd "$REPO_ROOT/.." && pwd)"
AXON_ROOT="${EASYNET_AXON_ROOT:-$WORKSPACE_ROOT/EasyNet-Axon}"
BRIDGE_ROOT="$AXON_ROOT/core/runtime-rs/dendrite-bridge"
CARGO_BIN="${CARGO_BIN:-cargo}"
OUT_DIR="${EASYNET_CLI_ARTIFACT_OUT_DIR:-}"
TARGET="${EASYNET_CLI_LINUX_TARGET:-}"
SELF_TEST=0

usage() {
  cat <<'USAGE'
Usage:
  tools/scripts/build-linux-cli-artifact-bundle.sh [options]

Options:
  --out-dir DIR      Destination artifact bundle directory.
  --target TRIPLE    Linux Rust target. Defaults to host-matching Linux GNU.
  --self-test        Validate script structure without compiling.
  -h, --help         Show this help.

Environment:
  EASYNET_CLI_ARTIFACT_OUT_DIR  Same as --out-dir.
  EASYNET_CLI_LINUX_TARGET      Same as --target.
  EASYNET_AXON_ROOT             Sibling EasyNet-Axon repository root.
  CARGO_BIN                     Cargo executable. Defaults to cargo.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    --target) TARGET="${2:?missing value for --target}"; shift 2 ;;
    --self-test) SELF_TEST=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

die() {
  echo "[FAIL] $*" >&2
  exit 1
}

host_default_target() {
  case "$(uname -m)" in
    arm64|aarch64) echo "aarch64-unknown-linux-gnu" ;;
    x86_64|amd64) echo "x86_64-unknown-linux-gnu" ;;
    *) return 1 ;;
  esac
}

TARGET="${TARGET:-$(host_default_target || true)}"
[[ -n "$TARGET" ]] || die "unsupported host arch: $(uname -m)"
case "$TARGET" in
  aarch64-unknown-linux-gnu|x86_64-unknown-linux-gnu) ;;
  *) die "unsupported Linux artifact target: $TARGET" ;;
esac

[[ -d "$BRIDGE_ROOT" ]] || die "EasyNet-Axon dendrite bridge not found: $BRIDGE_ROOT"
[[ -d "$REPO_ROOT/ability-descriptors/system" ]] || die "missing system ability descriptors"

if [[ "$SELF_TEST" == "1" ]]; then
  bash -n "$0"
  grep -q "cargo zigbuild" "$0"
  grep -q "libeasynet_cli.so" "$0"
  grep -q "libaxon_dendrite_bridge.so" "$0"
  grep -q "ability-descriptors" "$0"
  echo "build-linux-cli-artifact-bundle self-test ok"
  exit 0
fi

[[ -n "$OUT_DIR" ]] || die "--out-dir or EASYNET_CLI_ARTIFACT_OUT_DIR is required"
command -v "$CARGO_BIN" >/dev/null 2>&1 || die "missing cargo executable: $CARGO_BIN"
"$CARGO_BIN" zigbuild --help >/dev/null 2>&1 || die "cargo zigbuild is required"

mkdir -p "$OUT_DIR"

echo "==> building EasyNet CLI artifact bundle for $TARGET"
(
  cd "$REPO_ROOT"
  "$CARGO_BIN" zigbuild --release --target "$TARGET" --lib \
    --bin easynet --bin easynet-daemon --bin easynet-keyring \
    --no-default-features \
    --features axon-pb,demo-fixture
)

echo "==> building dendrite bridge artifact for $TARGET"
(
  cd "$BRIDGE_ROOT"
  "$CARGO_BIN" zigbuild --release --target "$TARGET"
)

rm -rf "$OUT_DIR.tmp"
mkdir -p "$OUT_DIR.tmp"
cp "$REPO_ROOT/target/$TARGET/release/easynet" "$OUT_DIR.tmp/easynet"
cp "$REPO_ROOT/target/$TARGET/release/easynet-daemon" "$OUT_DIR.tmp/easynet-daemon"
cp "$REPO_ROOT/target/$TARGET/release/easynet-keyring" "$OUT_DIR.tmp/easynet-keyring"
cp "$REPO_ROOT/target/$TARGET/release/libeasynet_cli.so" "$OUT_DIR.tmp/libeasynet_cli.so"
cp "$BRIDGE_ROOT/target/$TARGET/release/libaxon_dendrite_bridge.so" \
  "$OUT_DIR.tmp/libaxon_dendrite_bridge.so"
cp -R "$REPO_ROOT/ability-descriptors" "$OUT_DIR.tmp/ability-descriptors"

for required in \
  easynet \
  easynet-daemon \
  easynet-keyring \
  libeasynet_cli.so \
  libaxon_dendrite_bridge.so
do
  [[ -f "$OUT_DIR.tmp/$required" ]] || die "artifact missing after build: $required"
done
[[ -d "$OUT_DIR.tmp/ability-descriptors/system" ]] || die "artifact missing ability descriptors"

rm -rf "$OUT_DIR"
mv "$OUT_DIR.tmp" "$OUT_DIR"
echo "[OK] CLI artifact bundle: $OUT_DIR"
echo "  target: $TARGET"
