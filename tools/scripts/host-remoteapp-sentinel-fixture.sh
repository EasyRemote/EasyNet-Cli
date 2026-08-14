#!/usr/bin/env bash
# Launches macOS host sentinel windows for remoteapp decoded-frame E2E.
#
# Boundary:
# - This script owns only the visual host fixture: two visible native windows
#   with stable labels and distinct RGB sentinels.
# - It does not invoke EasyNet, create sessions, decode media, or assert pixels.
#   The decoded-frame E2E harness still performs those checks through the real
#   probe/receiver path.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

TARGET_KIND="${EASYNET_REMOTEAPP_E2E_TARGET_KIND:-window}"
OUT_DIR="${EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR:-}"
STOP=0

usage() {
  cat <<'USAGE'
Usage:
  tools/scripts/host-remoteapp-sentinel-fixture.sh [options]

Options:
  --target-kind KIND    Target kind: window or application. Default: window.
  --out-dir DIR         Fixture state directory. Required unless
                        EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR is set.
  --stop                Stop fixture processes recorded in --out-dir.
  -h, --help            Show this help.

Output:
  Writes:
    env.sh              Exported sentinel variables for the E2E harness.
    manifest.json       Fixture metadata and process ids.
    cleanup.sh          Idempotent cleanup command.

The fixture requires macOS and swiftc because it launches AppKit windows. It
creates two independent native processes so application-scoped E2E can prove
that another visible application is excluded from the decoded stream.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target-kind)
      case "${2:?missing value for --target-kind}" in
        window|application) TARGET_KIND="$2" ;;
        *) echo "invalid target kind: $2" >&2; exit 64 ;;
      esac
      shift 2
      ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    --stop) STOP=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

[[ -n "$OUT_DIR" ]] || {
  echo "[FAIL] --out-dir or EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR is required" >&2
  exit 64
}

PIDS_FILE="$OUT_DIR/pids"
ENV_FILE="$OUT_DIR/env.sh"
MANIFEST_JSON="$OUT_DIR/manifest.json"
CLEANUP_SH="$OUT_DIR/cleanup.sh"

stop_fixture() {
  if [[ -f "$PIDS_FILE" ]]; then
    while IFS= read -r pid; do
      [[ -n "$pid" ]] || continue
      if kill -0 "$pid" >/dev/null 2>&1; then
        kill "$pid" >/dev/null 2>&1 || true
      fi
    done <"$PIDS_FILE"
  fi
}

if [[ "$STOP" == "1" ]]; then
  stop_fixture
  exit 0
fi

[[ "$(uname -s)" == "Darwin" ]] || {
  echo "[FAIL] host sentinel fixture requires macOS/AppKit" >&2
  exit 1
}
command -v swiftc >/dev/null 2>&1 || {
  echo "[FAIL] host sentinel fixture requires swiftc" >&2
  exit 1
}

mkdir -p "$OUT_DIR"

FIXTURE_ID="$(date +%Y%m%d%H%M%S)-$$"
SELECTED_LABEL="EasyNet selected ${TARGET_KIND} sentinel ${FIXTURE_ID}"
UNRELATED_LABEL="EasyNet unrelated ${TARGET_KIND} sentinel ${FIXTURE_ID}"
SELECTED_RGB="255,0,0"
UNRELATED_RGB="0,255,0"
UNRELATED_PLACEMENT="other_window"
if [[ "$TARGET_KIND" == "application" ]]; then
  UNRELATED_PLACEMENT="other_application"
fi

SWIFT_SRC="$OUT_DIR/SentinelWindow.swift"
SELECTED_BIN="$OUT_DIR/easynet-remoteapp-selected-sentinel"
UNRELATED_BIN="$OUT_DIR/easynet-remoteapp-unrelated-sentinel"

cat >"$SWIFT_SRC" <<'SWIFT'
import AppKit

final class SentinelView: NSView {
    let color: NSColor
    let label: String

    init(frame frameRect: NSRect, color: NSColor, label: String) {
        self.color = color
        self.label = label
        super.init(frame: frameRect)
        wantsLayer = true
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func draw(_ dirtyRect: NSRect) {
        color.setFill()
        dirtyRect.fill()
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = .center
        let attributes: [NSAttributedString.Key: Any] = [
            .foregroundColor: NSColor.white,
            .font: NSFont.monospacedSystemFont(ofSize: 24, weight: .bold),
            .paragraphStyle: paragraph,
        ]
        let rect = NSRect(x: 20, y: bounds.midY - 40, width: bounds.width - 40, height: 80)
        label.draw(in: rect, withAttributes: attributes)
    }
}

func parseByte(_ value: String) -> CGFloat {
    guard let intValue = Int(value), intValue >= 0, intValue <= 255 else {
        fputs("invalid RGB byte: \(value)\n", stderr)
        exit(64)
    }
    return CGFloat(intValue) / 255.0
}

let args = CommandLine.arguments
guard args.count == 8 else {
    fputs("usage: SentinelWindow <title> <r,g,b> <x> <y> <width> <height> <activation>\n", stderr)
    exit(64)
}

let title = args[1]
let rgb = args[2].split(separator: ",").map(String.init)
guard rgb.count == 3 else {
    fputs("RGB must be formatted as r,g,b\n", stderr)
    exit(64)
}

let x = Double(args[3]) ?? 80
let y = Double(args[4]) ?? 120
let width = Double(args[5]) ?? 420
let height = Double(args[6]) ?? 260
let activation = args[7]

let app = NSApplication.shared
app.setActivationPolicy(.regular)

let color = NSColor(
    calibratedRed: parseByte(rgb[0]),
    green: parseByte(rgb[1]),
    blue: parseByte(rgb[2]),
    alpha: 1.0
)
let rect = NSRect(x: x, y: y, width: width, height: height)
let window = NSWindow(
    contentRect: rect,
    styleMask: [.titled, .closable, .resizable],
    backing: .buffered,
    defer: false
)
window.title = title
window.contentView = SentinelView(frame: NSRect(x: 0, y: 0, width: width, height: height), color: color, label: title)
window.isReleasedWhenClosed = false
window.orderFrontRegardless()
if activation == "activate" {
    app.activate(ignoringOtherApps: true)
}
app.run()
SWIFT

swiftc "$SWIFT_SRC" -framework AppKit -o "$SELECTED_BIN"
cp "$SELECTED_BIN" "$UNRELATED_BIN"
chmod +x "$SELECTED_BIN" "$UNRELATED_BIN"

rm -f "$PIDS_FILE"
"$SELECTED_BIN" "$SELECTED_LABEL" "$SELECTED_RGB" 80 160 460 300 activate \
  >"$OUT_DIR/selected.log" 2>&1 &
SELECTED_PID="$!"
printf '%s\n' "$SELECTED_PID" >>"$PIDS_FILE"

"$UNRELATED_BIN" "$UNRELATED_LABEL" "$UNRELATED_RGB" 620 160 460 300 activate \
  >"$OUT_DIR/unrelated.log" 2>&1 &
UNRELATED_PID="$!"
printf '%s\n' "$UNRELATED_PID" >>"$PIDS_FILE"

sleep 2
for pid in "$SELECTED_PID" "$UNRELATED_PID"; do
  if ! kill -0 "$pid" >/dev/null 2>&1; then
    stop_fixture
    echo "[FAIL] sentinel fixture process exited early: $pid" >&2
    exit 1
  fi
done

python3 - "$ENV_FILE" "$MANIFEST_JSON" "$CLEANUP_SH" "$OUT_DIR" "$REPO_ROOT" \
  "$TARGET_KIND" "$SELECTED_LABEL" "$UNRELATED_LABEL" "$SELECTED_RGB" "$UNRELATED_RGB" \
  "$UNRELATED_PLACEMENT" "$SELECTED_PID" "$UNRELATED_PID" <<'PY'
import json
import shlex
import sys

(
    env_path,
    manifest_path,
    cleanup_path,
    out_dir,
    repo_root,
    target_kind,
    selected_label,
    unrelated_label,
    selected_rgb,
    unrelated_rgb,
    unrelated_placement,
    selected_pid,
    unrelated_pid,
) = sys.argv[1:14]

exports = {
    "EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB": selected_rgb,
    "EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB": unrelated_rgb,
    "EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL": selected_label,
    "EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL": unrelated_label,
    "EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PLACEMENT": unrelated_placement,
    "EASYNET_REMOTEAPP_TARGET_HINT": selected_label,
    "EASYNET_REMOTEAPP_TARGET_PID": selected_pid,
    "EASYNET_REMOTEAPP_SELECTED_SENTINEL_PID": selected_pid,
    "EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PID": unrelated_pid,
    "EASYNET_REMOTEAPP_SENTINEL_FIXTURE_MANIFEST": manifest_path,
}

with open(env_path, "w", encoding="utf-8") as f:
    for key, value in exports.items():
        f.write(f"export {key}={shlex.quote(value)}\n")

manifest = {
    "target_kind": target_kind,
    "proof": "dual_target_non_leak",
    "selected": {
        "label": selected_label,
        "rgb": [int(part) for part in selected_rgb.split(",")],
        "pid": int(selected_pid),
    },
    "unrelated": {
        "label": unrelated_label,
        "rgb": [int(part) for part in unrelated_rgb.split(",")],
        "pid": int(unrelated_pid),
        "placement": unrelated_placement,
    },
}
with open(manifest_path, "w", encoding="utf-8") as f:
    json.dump(manifest, f, indent=2, sort_keys=True)
    f.write("\n")

with open(cleanup_path, "w", encoding="utf-8") as f:
    f.write("#!/usr/bin/env bash\n")
    f.write("set -euo pipefail\n")
    f.write(f"{shlex.quote(repo_root + '/tools/scripts/host-remoteapp-sentinel-fixture.sh')} --stop --out-dir {shlex.quote(out_dir)}\n")
PY
chmod +x "$CLEANUP_SH"

printf 'host-remoteapp-sentinel-fixture: started selected_pid=%s unrelated_pid=%s env=%s\n' \
  "$SELECTED_PID" "$UNRELATED_PID" "$ENV_FILE"
