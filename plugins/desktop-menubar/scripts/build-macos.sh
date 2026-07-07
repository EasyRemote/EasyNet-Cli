#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
PACKAGE_ROOT="${EASYNET_DESKTOP_MENUBAR_PACKAGE_ROOT:-$ROOT/plugins/desktop-menubar}"
APP_DIR="$PACKAGE_ROOT/dist/macos/EasyNetMenuBar.app"
TARGET_APP_DIR="${EASYNET_DESKTOP_MENUBAR_TARGET_APP_DIR:-$ROOT/target/macos/EasyNetMenuBar.app}"
CONTENTS="$APP_DIR/Contents"
MACOS="$CONTENTS/MacOS"
RESOURCES="$CONTENTS/Resources"
SRC="$ROOT/plugins/desktop-menubar/companion/macos/EasyNetMenuBar/Sources/EasyNetMenuBar/main.swift"
SOURCE_RESOURCES="$ROOT/plugins/desktop-menubar/companion/macos/EasyNetMenuBar/Resources"

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
cp -R "$SOURCE_RESOURCES"/. "$RESOURCES"/
codesign --force --sign - "$APP_DIR" >/dev/null
rm -rf "$TARGET_APP_DIR"
mkdir -p "$(dirname "$TARGET_APP_DIR")"
cp -R "$APP_DIR" "$TARGET_APP_DIR"

echo "$APP_DIR"
