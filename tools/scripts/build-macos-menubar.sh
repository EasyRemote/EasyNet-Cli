#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
APP_DIR="$ROOT/target/macos/EasyNetMenuBar.app"
CONTENTS="$APP_DIR/Contents"
MACOS="$CONTENTS/MacOS"
RESOURCES="$CONTENTS/Resources"
SRC="$ROOT/platforms/macos/EasyNetMenuBar/Sources/EasyNetMenuBar/main.swift"
ICON_SRC="$ROOT/platforms/macos/EasyNetMenuBar/Resources/easynet-template.png"

rm -rf "$APP_DIR"
mkdir -p "$MACOS" "$RESOURCES"

swiftc \
  -O \
  -framework AppKit \
  -framework Carbon \
  -framework CryptoKit \
  "$SRC" \
  -o "$MACOS/EasyNetMenuBar"

cp "$ICON_SRC" "$RESOURCES/easynet-template.png"
sips -Z 36 "$ICON_SRC" --out "$RESOURCES/easynet-status.png" >/dev/null
cp "$ROOT/platforms/macos/EasyNetMenuBar/Info.plist" "$CONTENTS/Info.plist"

echo "$APP_DIR"
