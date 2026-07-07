#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
APP_DIR="$ROOT/plugins/desktop-menubar/dist/macos/EasyNetMenuBar.app"
TARGET_APP_DIR="$ROOT/target/macos/EasyNetMenuBar.app"
CONTENTS="$APP_DIR/Contents"
MACOS="$CONTENTS/MacOS"
RESOURCES="$CONTENTS/Resources"
SRC="$ROOT/plugins/desktop-menubar/companion/macos/EasyNetMenuBar/Sources/EasyNetMenuBar/main.swift"

rm -rf "$APP_DIR"
mkdir -p "$MACOS" "$RESOURCES"

swiftc \
  -O \
  -framework AppKit \
  -framework Carbon \
  -framework CryptoKit \
  "$SRC" \
  -o "$MACOS/EasyNetMenuBar"

cp "$ROOT/plugins/desktop-menubar/companion/macos/EasyNetMenuBar/Info.plist" "$CONTENTS/Info.plist"
rm -rf "$TARGET_APP_DIR"
mkdir -p "$(dirname "$TARGET_APP_DIR")"
cp -R "$APP_DIR" "$TARGET_APP_DIR"

echo "$APP_DIR"
