#!/usr/bin/env bash
# vendor.sh — copy EasyNet-Axon SDK into the build context.
#
# Run this BEFORE `docker compose build`. The Dockerfile's path-dep
# rewrite expects `vendor/easynet-axon-sdk-rust/` to exist with the
# axon SDK Rust source, sans `target/` and `.git/`. We do NOT track
# the vendored copy in git (see .gitignore) — it would duplicate
# upstream and bloat history.

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
SRC="${EASYNET_AXON_PATH:-${HERE}/../../../../../EasyNet-Axon/sdk/rust}"
DEST="${HERE}/vendor/easynet-axon-sdk-rust"

if [ ! -d "$SRC" ]; then
    echo "[vendor.sh] EasyNet-Axon SDK Rust dir not found at: $SRC" >&2
    echo "[vendor.sh] set EASYNET_AXON_PATH to override" >&2
    exit 1
fi

mkdir -p "$(dirname "$DEST")"
rm -rf "$DEST"
rsync -a \
    --exclude='target/' \
    --exclude='.git/' \
    --exclude='node_modules/' \
    "$SRC/" "$DEST/"

echo "[vendor.sh] vendored $SRC -> $DEST  ($(du -sh "$DEST" | cut -f1))"
