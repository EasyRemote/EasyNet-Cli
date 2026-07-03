#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
APP_NAME="EasyNetMenuBar.app"
SOURCE_APP="$ROOT/target/macos/$APP_NAME"
INSTALL_DIR="$HOME/.easynet/apps"
INSTALL_APP="$INSTALL_DIR/$APP_NAME"
LAUNCH_AGENTS="$HOME/Library/LaunchAgents"
PLIST="$LAUNCH_AGENTS/tech.silan.easynet.menubar.plist"

"$ROOT/tools/scripts/build-macos-menubar.sh" >/dev/null

mkdir -p "$INSTALL_DIR" "$LAUNCH_AGENTS"
rm -rf "$INSTALL_APP"
cp -R "$SOURCE_APP" "$INSTALL_APP"

cat > "$PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>tech.silan.easynet.menubar</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/bin/open</string>
    <string>-g</string>
    <string>$INSTALL_APP</string>
  </array>
  <key>LimitLoadToSessionType</key>
  <string>Aqua</string>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <false/>
  <key>StandardOutPath</key>
  <string>$HOME/.easynet/easynet-menubar.out.log</string>
  <key>StandardErrorPath</key>
  <string>$HOME/.easynet/easynet-menubar.err.log</string>
</dict>
</plist>
PLIST

launchctl bootout "gui/$(id -u)" "$PLIST" >/dev/null 2>&1 || true
launchctl bootstrap "gui/$(id -u)" "$PLIST"
launchctl kickstart -k "gui/$(id -u)/tech.silan.easynet.menubar"

echo "$INSTALL_APP"
