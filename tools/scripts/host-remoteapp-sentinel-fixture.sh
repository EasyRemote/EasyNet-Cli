#!/usr/bin/env bash
# Launches macOS host sentinel windows for remoteapp decoded-frame E2E.
#
# Boundary:
# - This script owns only the visual host fixture: one selected target and one
#   unrelated target with stable labels and distinct RGB sentinels. Application
#   mode gives the selected process two independently colored native windows.
# - It does not invoke EasyNet, create sessions, decode media, or assert pixels.
#   The decoded-frame E2E harness still performs those checks through the real
#   probe/receiver path.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

TARGET_KIND="${EASYNET_REMOTEAPP_E2E_TARGET_KIND:-window}"
OUT_DIR="${EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR:-}"
ENABLE_AUDIO_TONE=0
STOP=0

usage() {
  cat <<'USAGE'
Usage:
  tools/scripts/host-remoteapp-sentinel-fixture.sh [options]

Options:
  --target-kind KIND    Target kind: window or application. Default: window.
  --out-dir DIR         Fixture state directory. Required unless
                        EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR is set.
  --audio-tone          In application mode, emit distinct selected and
                        unrelated process-owned tones for host-audio E2E.
  --stop                Stop fixture processes recorded in --out-dir.
  -h, --help            Show this help.

Output:
  Writes:
    env.sh              Exported sentinel variables for the E2E harness.
    manifest.json       Fixture metadata and process ids.
    selected-control.sh  Control helper for moving/resizing/closing the selected
                        sentinel window during lifecycle E2E scenarios.
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
    --audio-tone) ENABLE_AUDIO_TONE=1; shift ;;
    --stop) STOP=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

[[ -n "$OUT_DIR" ]] || {
  echo "[FAIL] --out-dir or EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR is required" >&2
  exit 64
}
if [[ "$ENABLE_AUDIO_TONE" == "1" && "$TARGET_KIND" != "application" ]]; then
  echo "[FAIL] --audio-tone requires --target-kind application" >&2
  exit 64
fi

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd -P)"

PIDS_FILE="$OUT_DIR/pids"
ENV_FILE="$OUT_DIR/env.sh"
MANIFEST_JSON="$OUT_DIR/manifest.json"
CLEANUP_SH="$OUT_DIR/cleanup.sh"
RUNTIME_DIR_FILE="$OUT_DIR/runtime-dir"

stop_fixture() {
  local runtime_dir=""
  if [[ -f "$RUNTIME_DIR_FILE" ]]; then
    runtime_dir="$(sed -n '1p' "$RUNTIME_DIR_FILE")"
  fi
  local owned_pids=""
  if [[ -f "$PIDS_FILE" ]]; then
    while IFS= read -r pid; do
      [[ -n "$pid" ]] || continue
      case "$pid" in
        *[!0-9]*) continue ;;
      esac
      local command_line=""
      command_line="$(ps -p "$pid" -o command= 2>/dev/null || true)"
      if [[ -n "$runtime_dir" && "$command_line" == *"$runtime_dir/"* ]] \
          && kill -0 "$pid" >/dev/null 2>&1; then
        kill "$pid" >/dev/null 2>&1 || true
        owned_pids="$owned_pids $pid"
      fi
    done <"$PIDS_FILE"
  fi
  local attempt pid
  for attempt in {1..50}; do
    local running=0
    for pid in $owned_pids; do
      if kill -0 "$pid" >/dev/null 2>&1; then
        running=1
      fi
    done
    [[ "$running" -eq 0 ]] && break
    sleep 0.02
  done
  for pid in $owned_pids; do
    if kill -0 "$pid" >/dev/null 2>&1; then
      kill -KILL "$pid" >/dev/null 2>&1 || true
    fi
  done
  local runtime_parent runtime_name temp_root
  runtime_parent="$(dirname "$runtime_dir")"
  runtime_name="$(basename "$runtime_dir")"
  temp_root="$(cd "${TMPDIR:-/tmp}" && pwd -P)"
  if [[ -n "$runtime_dir" && -d "$runtime_dir" \
      && "$runtime_name" == easynet-remoteapp-sentinel.* \
      && "$runtime_parent" == "$temp_root" ]]; then
    rm -rf -- "$runtime_dir"
  fi
}

FIXTURE_HANDOFF_COMPLETE=0
cleanup_before_handoff() {
  local exit_code=$?
  if [[ "$FIXTURE_HANDOFF_COMPLETE" -ne 1 ]]; then
    stop_fixture
  fi
  return "$exit_code"
}
trap cleanup_before_handoff EXIT

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

# LaunchServices-started fixtures do not inherit terminal privacy access to a
# repository under ~/Documents. Keep executable bundles and their control/event
# IPC in a fixture-owned temporary directory; durable manifests and reports
# remain in OUT_DIR. The exact runtime path is recorded for bounded cleanup.
stop_fixture
TEMP_ROOT="$(cd "${TMPDIR:-/tmp}" && pwd -P)"
RUNTIME_DIR="$(mktemp -d "$TEMP_ROOT/easynet-remoteapp-sentinel.XXXXXX")"
printf '%s\n' "$RUNTIME_DIR" >"$RUNTIME_DIR_FILE"

FIXTURE_ID="$(date +%Y%m%d%H%M%S)-$$"
SELECTED_LABEL="EasyNet selected ${TARGET_KIND} sentinel ${FIXTURE_ID}"
UNRELATED_LABEL="EasyNet unrelated ${TARGET_KIND} sentinel ${FIXTURE_ID}"
SELECTED_RGB="255,0,0"
SELECTED_SECONDARY_RGB=""
UNRELATED_RGB="0,255,0"
SELECTED_AUDIO_FREQUENCY_HZ=""
UNRELATED_AUDIO_FREQUENCY_HZ=""
UNRELATED_PLACEMENT="other_window"
if [[ "$TARGET_KIND" == "application" ]]; then
  UNRELATED_PLACEMENT="other_application"
  SELECTED_SECONDARY_RGB="0,0,255"
fi
if [[ "$ENABLE_AUDIO_TONE" == "1" ]]; then
  SELECTED_AUDIO_FREQUENCY_HZ="523.25"
  UNRELATED_AUDIO_FREQUENCY_HZ="880.0"
fi

SWIFT_SRC="$RUNTIME_DIR/SentinelWindow.swift"
SELECTED_BIN="$RUNTIME_DIR/easynet-remoteapp-selected-sentinel"
UNRELATED_BIN="$RUNTIME_DIR/easynet-remoteapp-unrelated-sentinel"
SELECTED_BUNDLE_ID=""
UNRELATED_BUNDLE_ID=""

cat >"$SWIFT_SRC" <<'SWIFT'
import AppKit
import AVFoundation

final class SentinelView: NSView {
    let color: NSColor
    let label: String
    let eventPath: String

    init(frame frameRect: NSRect, color: NSColor, label: String, eventPath: String) {
        self.color = color
        self.label = label
        self.eventPath = eventPath
        super.init(frame: frameRect)
        wantsLayer = true
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override var acceptsFirstResponder: Bool { true }

    private func appendEvent(kind: String, action: String, event: NSEvent, keyCode: String? = nil) {
        let point = event.cgEvent?.location ?? .zero
        var record: [String: Any] = [
            "kind": kind,
            "action": action,
            "label": label,
            "pid": ProcessInfo.processInfo.processIdentifier,
            "observed_at_ms": Int64((Date().timeIntervalSince1970 * 1000.0).rounded()),
            "global_position": ["x": point.x, "y": point.y],
        ]
        if let keyCode {
            record["key_code"] = keyCode
        }
        guard let data = try? JSONSerialization.data(withJSONObject: record),
              var line = String(data: data, encoding: .utf8) else {
            return
        }
        line.append("\n")
        if !FileManager.default.fileExists(atPath: eventPath) {
            FileManager.default.createFile(atPath: eventPath, contents: nil)
        }
        guard let handle = FileHandle(forWritingAtPath: eventPath) else { return }
        defer { try? handle.close() }
        do {
            try handle.seekToEnd()
            if let bytes = line.data(using: .utf8) {
                try handle.write(contentsOf: bytes)
            }
        } catch {
            fputs("event log write failed: \(error)\n", stderr)
        }
    }

    override func mouseDown(with event: NSEvent) {
        appendEvent(kind: "pointer", action: "down", event: event)
    }

    override func mouseUp(with event: NSEvent) {
        appendEvent(kind: "pointer", action: "up", event: event)
    }

    override func keyDown(with event: NSEvent) {
        let code = event.keyCode == 0 ? "KeyA" : "MacKeyCode\(event.keyCode)"
        appendEvent(kind: "keyboard", action: "down", event: event, keyCode: code)
    }

    override func keyUp(with event: NSEvent) {
        let code = event.keyCode == 0 ? "KeyA" : "MacKeyCode\(event.keyCode)"
        appendEvent(kind: "keyboard", action: "up", event: event, keyCode: code)
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

final class TonePhase: @unchecked Sendable {
    var value = 0.0
}

final class ToneGenerator {
    private let engine = AVAudioEngine()
    private let source: AVAudioSourceNode

    init(frequencyHz: Double) throws {
        guard frequencyHz.isFinite && frequencyHz > 0.0 && frequencyHz < 24_000.0 else {
            throw NSError(
                domain: "EasyNetSentinelTone",
                code: 64,
                userInfo: [NSLocalizedDescriptionKey: "invalid tone frequency \(frequencyHz)"]
            )
        }
        let sampleRate = 48_000.0
        let phase = TonePhase()
        let phaseStep = 2.0 * Double.pi * frequencyHz / sampleRate
        guard let format = AVAudioFormat(
            standardFormatWithSampleRate: sampleRate,
            channels: 2
        ) else {
            throw NSError(
                domain: "EasyNetSentinelTone",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: "cannot create 48 kHz stereo format"]
            )
        }
        source = AVAudioSourceNode(format: format) {
            _, _, frameCount, audioBufferList -> OSStatus in
            let buffers = UnsafeMutableAudioBufferListPointer(audioBufferList)
            for frame in 0..<Int(frameCount) {
                let sample = Float(sin(phase.value)) * 0.16
                phase.value += phaseStep
                if phase.value >= 2.0 * Double.pi {
                    phase.value -= 2.0 * Double.pi
                }
                for buffer in buffers {
                    guard let data = buffer.mData else { continue }
                    let channels = Int(buffer.mNumberChannels)
                    let samples = data.assumingMemoryBound(to: Float.self)
                    for channel in 0..<channels {
                        samples[frame * channels + channel] = sample
                    }
                }
            }
            return noErr
        }
        engine.attach(source)
        engine.connect(source, to: engine.mainMixerNode, format: format)
        engine.prepare()
        try engine.start()
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
if args.count == 6 && args[1] == "--probe-window-counts" {
    guard let selectedPid = Int32(args[2]),
          let selectedExpected = Int(args[3]),
          let unrelatedPid = Int32(args[4]),
          let unrelatedExpected = Int(args[5]),
          let rows = CGWindowListCopyWindowInfo(
              [.optionOnScreenOnly, .excludeDesktopElements],
              kCGNullWindowID
          ) as? [[String: Any]] else {
        exit(1)
    }
    func layerZeroCount(for pid: Int32) -> Int {
        rows.filter { row in
            guard let owner = row[kCGWindowOwnerPID as String] as? NSNumber,
                  let layer = row[kCGWindowLayer as String] as? NSNumber else {
                return false
            }
            return owner.int32Value == pid && layer.intValue == 0
        }.count
    }
    let selectedCount = layerZeroCount(for: selectedPid)
    let unrelatedCount = layerZeroCount(for: unrelatedPid)
    if selectedCount >= selectedExpected && unrelatedCount >= unrelatedExpected {
        exit(0)
    }
    fputs(
        "window counts not ready: selected=\(selectedCount)/\(selectedExpected) unrelated=\(unrelatedCount)/\(unrelatedExpected)\n",
        stderr
    )
    exit(1)
}
guard args.count == 13 else {
    fputs("usage: SentinelWindow <title> <r,g,b> <x> <y> <width> <height> <activation> <command-file> <ack-file> <event-log> <secondary-rgb|none> <tone-frequency-hz|none>\n", stderr)
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
let commandPath = args[8]
let ackPath = args[9]
let eventPath = args[10]
let secondaryRgbRaw = args[11]
let toneFrequencyRaw = args[12]

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
let sentinelView = SentinelView(
    frame: NSRect(x: 0, y: 0, width: width, height: height),
    color: color,
    label: title,
    eventPath: eventPath
)
window.contentView = sentinelView
window.isReleasedWhenClosed = false
window.acceptsMouseMovedEvents = true
window.orderFrontRegardless()

var secondaryWindow: NSWindow?
if secondaryRgbRaw != "none" {
    let secondaryRgb = secondaryRgbRaw.split(separator: ",").map(String.init)
    guard secondaryRgb.count == 3 else {
        fputs("secondary RGB must be formatted as r,g,b or none\n", stderr)
        exit(64)
    }
    let secondaryWidth = max(280.0, width * 0.74)
    let secondaryHeight = max(180.0, height * 0.72)
    let secondaryRect = NSRect(
        x: x + width * 0.42,
        y: y + height + 40.0,
        width: secondaryWidth,
        height: secondaryHeight
    )
    let second = NSWindow(
        contentRect: secondaryRect,
        styleMask: [.titled, .closable, .resizable],
        backing: .buffered,
        defer: false
    )
    let secondaryLabel = title + " secondary"
    second.title = secondaryLabel
    second.contentView = SentinelView(
        frame: NSRect(x: 0, y: 0, width: secondaryWidth, height: secondaryHeight),
        color: NSColor(
            calibratedRed: parseByte(secondaryRgb[0]),
            green: parseByte(secondaryRgb[1]),
            blue: parseByte(secondaryRgb[2]),
            alpha: 1.0
        ),
        label: secondaryLabel,
        eventPath: eventPath
    )
    second.isReleasedWhenClosed = false
    second.acceptsMouseMovedEvents = true
    second.orderFrontRegardless()
    secondaryWindow = second
}
if activation == "activate" {
    app.activate(ignoringOtherApps: true)
    window.makeKeyAndOrderFront(nil)
    window.makeFirstResponder(sentinelView)
}

var toneGenerator: ToneGenerator?
if toneFrequencyRaw != "none" {
    guard let frequencyHz = Double(toneFrequencyRaw) else {
        fputs("tone frequency must be a number or none\n", stderr)
        exit(64)
    }
    do {
        toneGenerator = try ToneGenerator(frequencyHz: frequencyHz)
    } catch {
        fputs("tone generator start failed: \(error)\n", stderr)
        exit(1)
    }
}

var lastCommand = ""
Timer.scheduledTimer(withTimeInterval: 0.10, repeats: true) { _ in
    guard let raw = try? String(contentsOfFile: commandPath, encoding: .utf8) else {
        return
    }
    let command = raw.trimmingCharacters(in: .whitespacesAndNewlines)
    if command.isEmpty || command == lastCommand {
        return
    }
    lastCommand = command
    let parts = command.split(separator: " ").map(String.init)
    guard let action = parts.first else {
        return
    }
    if action == "move", parts.count == 3,
       let nextX = Double(parts[1]),
       let nextY = Double(parts[2]) {
        let current = window.frame
        let next = NSRect(x: nextX, y: nextY, width: current.width, height: current.height)
        window.setFrame(next, display: true, animate: false)
        try? "move".write(toFile: ackPath, atomically: true, encoding: .utf8)
    } else if action == "move_resize", parts.count == 5,
       let nextX = Double(parts[1]),
       let nextY = Double(parts[2]),
       let nextWidth = Double(parts[3]),
       let nextHeight = Double(parts[4]) {
        let next = NSRect(x: nextX, y: nextY, width: nextWidth, height: nextHeight)
        window.setFrame(next, display: true, animate: false)
        window.contentView?.frame = NSRect(x: 0, y: 0, width: nextWidth, height: nextHeight)
        try? "move_resize".write(toFile: ackPath, atomically: true, encoding: .utf8)
    } else if action == "focus" {
        app.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
        window.makeFirstResponder(sentinelView)
        try? "focus".write(toFile: ackPath, atomically: true, encoding: .utf8)
    } else if action == "close" {
        try? "close".write(toFile: ackPath, atomically: true, encoding: .utf8)
        window.close()
        app.terminate(nil)
    } else {
        try? "unknown".write(toFile: ackPath, atomically: true, encoding: .utf8)
    }
}
app.run()
SWIFT

swiftc "$SWIFT_SRC" -framework AppKit -framework AVFoundation -o "$SELECTED_BIN"
cp "$SELECTED_BIN" "$UNRELATED_BIN"
chmod +x "$SELECTED_BIN" "$UNRELATED_BIN"

SELECTED_EXEC="$SELECTED_BIN"
UNRELATED_EXEC="$UNRELATED_BIN"
if [[ "$TARGET_KIND" == "application" ]]; then
  command -v plutil >/dev/null 2>&1 || {
    echo "[FAIL] application sentinel fixture requires plutil" >&2
    exit 1
  }
  FIXTURE_BUNDLE_TOKEN="$(printf '%s' "$FIXTURE_ID" | tr -cd '[:alnum:]')"
  SELECTED_BUNDLE_ID="tech.silan.easynet.remoteapp.selectedsentinel.$FIXTURE_BUNDLE_TOKEN"
  UNRELATED_BUNDLE_ID="tech.silan.easynet.remoteapp.unrelatedsentinel.$FIXTURE_BUNDLE_TOKEN"
  SELECTED_APP="$RUNTIME_DIR/EasyNetSelectedSentinel.app"
  UNRELATED_APP="$RUNTIME_DIR/EasyNetUnrelatedSentinel.app"
  SELECTED_EXEC="$SELECTED_APP/Contents/MacOS/EasyNetSelectedSentinel"
  UNRELATED_EXEC="$UNRELATED_APP/Contents/MacOS/EasyNetUnrelatedSentinel"
  mkdir -p "$(dirname "$SELECTED_EXEC")" "$(dirname "$UNRELATED_EXEC")"
  cp "$SELECTED_BIN" "$SELECTED_EXEC"
  cp "$UNRELATED_BIN" "$UNRELATED_EXEC"
  chmod +x "$SELECTED_EXEC" "$UNRELATED_EXEC"
  for bundle_spec in \
    "$SELECTED_APP|$SELECTED_BUNDLE_ID|EasyNetSelectedSentinel|EasyNet Selected Sentinel" \
    "$UNRELATED_APP|$UNRELATED_BUNDLE_ID|EasyNetUnrelatedSentinel|EasyNet Unrelated Sentinel"; do
    IFS='|' read -r app_path bundle_id executable_name bundle_name <<<"$bundle_spec"
    plist="$app_path/Contents/Info.plist"
    plutil -create xml1 "$plist"
    plutil -insert CFBundleIdentifier -string "$bundle_id" "$plist"
    plutil -insert CFBundleExecutable -string "$executable_name" "$plist"
    plutil -insert CFBundleName -string "$bundle_name" "$plist"
    plutil -insert CFBundleDisplayName -string "$bundle_name" "$plist"
    plutil -insert CFBundlePackageType -string APPL "$plist"
  done
fi

SELECTED_COMMAND_FILE="$RUNTIME_DIR/selected-command.txt"
SELECTED_ACK_FILE="$RUNTIME_DIR/selected-ack.txt"
UNRELATED_COMMAND_FILE="$RUNTIME_DIR/unrelated-command.txt"
UNRELATED_ACK_FILE="$RUNTIME_DIR/unrelated-ack.txt"
SELECTED_CONTROL_SH="$OUT_DIR/selected-control.sh"
SELECTED_EVENT_LOG="$RUNTIME_DIR/selected-input-events.jsonl"
UNRELATED_EVENT_LOG="$RUNTIME_DIR/unrelated-input-events.jsonl"
rm -f "$PIDS_FILE" "$SELECTED_COMMAND_FILE" "$SELECTED_ACK_FILE" \
  "$UNRELATED_COMMAND_FILE" "$UNRELATED_ACK_FILE" \
  "$SELECTED_EVENT_LOG" "$UNRELATED_EVENT_LOG"
SELECTED_TONE_ARG="${SELECTED_AUDIO_FREQUENCY_HZ:-none}"
UNRELATED_TONE_ARG="${UNRELATED_AUDIO_FREQUENCY_HZ:-none}"
if [[ "$TARGET_KIND" == "application" ]]; then
  open -n "$SELECTED_APP" --args \
    "$SELECTED_LABEL" "$SELECTED_RGB" 80 160 460 300 activate "$SELECTED_COMMAND_FILE" "$SELECTED_ACK_FILE" \
    "$SELECTED_EVENT_LOG" "$SELECTED_SECONDARY_RGB" "$SELECTED_TONE_ARG" >"$OUT_DIR/selected.log" 2>&1
  for _ in {1..100}; do
    SELECTED_PID="$(pgrep -f "$SELECTED_EXEC" | head -n 1 || true)"
    [[ -n "$SELECTED_PID" ]] && break
    sleep 0.05
  done
  [[ -n "${SELECTED_PID:-}" ]] || {
    echo "[FAIL] LaunchServices did not publish selected application PID" >&2
    exit 1
  }
else
  "$SELECTED_EXEC" "$SELECTED_LABEL" "$SELECTED_RGB" 80 160 460 300 activate "$SELECTED_COMMAND_FILE" "$SELECTED_ACK_FILE" \
    "$SELECTED_EVENT_LOG" none "$SELECTED_TONE_ARG" \
    >"$OUT_DIR/selected.log" 2>&1 &
  SELECTED_PID="$!"
fi
printf '%s\n' "$SELECTED_PID" >>"$PIDS_FILE"

if [[ "$TARGET_KIND" == "application" ]]; then
  open -n "$UNRELATED_APP" --args \
    "$UNRELATED_LABEL" "$UNRELATED_RGB" 620 160 460 300 activate "$UNRELATED_COMMAND_FILE" "$UNRELATED_ACK_FILE" \
    "$UNRELATED_EVENT_LOG" none "$UNRELATED_TONE_ARG" >"$OUT_DIR/unrelated.log" 2>&1
  for _ in {1..100}; do
    UNRELATED_PID="$(pgrep -f "$UNRELATED_EXEC" | head -n 1 || true)"
    [[ -n "$UNRELATED_PID" ]] && break
    sleep 0.05
  done
  [[ -n "${UNRELATED_PID:-}" ]] || {
    stop_fixture
    echo "[FAIL] LaunchServices did not publish unrelated application PID" >&2
    exit 1
  }
else
  "$UNRELATED_EXEC" "$UNRELATED_LABEL" "$UNRELATED_RGB" 620 160 460 300 activate "$UNRELATED_COMMAND_FILE" "$UNRELATED_ACK_FILE" \
    "$UNRELATED_EVENT_LOG" none "$UNRELATED_TONE_ARG" \
    >"$OUT_DIR/unrelated.log" 2>&1 &
  UNRELATED_PID="$!"
fi
printf '%s\n' "$UNRELATED_PID" >>"$PIDS_FILE"

for pid in "$SELECTED_PID" "$UNRELATED_PID"; do
  if ! kill -0 "$pid" >/dev/null 2>&1; then
    stop_fixture
    echo "[FAIL] sentinel fixture process exited early: $pid" >&2
    exit 1
  fi
done

SELECTED_EXPECTED_WINDOWS=1
[[ "$TARGET_KIND" == "application" ]] && SELECTED_EXPECTED_WINDOWS=2
WINDOWS_READY=0
for _ in {1..200}; do
  if "$SELECTED_BIN" --probe-window-counts \
    "$SELECTED_PID" "$SELECTED_EXPECTED_WINDOWS" "$UNRELATED_PID" 1 \
    >"$OUT_DIR/window-readiness.stdout.txt" \
    2>"$OUT_DIR/window-readiness.stderr.txt"; then
    WINDOWS_READY=1
    break
  fi
  sleep 0.05
done
if [[ "$WINDOWS_READY" -ne 1 ]]; then
  stop_fixture
  echo "[FAIL] sentinel fixture windows did not reach CoreGraphics inventory readiness" >&2
  cat "$OUT_DIR/window-readiness.stderr.txt" >&2
  exit 1
fi

rm -f "$SELECTED_ACK_FILE"
printf 'focus\n' >"$SELECTED_COMMAND_FILE"
python3 - "$SELECTED_ACK_FILE" <<'PY'
import pathlib, sys, time
ack = pathlib.Path(sys.argv[1])
deadline = time.time() + 5.0
while time.time() < deadline:
    if ack.exists() and ack.read_text(encoding="utf-8").strip() == "focus":
        raise SystemExit(0)
    time.sleep(0.05)
raise SystemExit("selected sentinel did not acknowledge focus")
PY

python3 - "$ENV_FILE" "$MANIFEST_JSON" "$CLEANUP_SH" "$SELECTED_CONTROL_SH" "$OUT_DIR" "$REPO_ROOT" \
  "$TARGET_KIND" "$SELECTED_LABEL" "$UNRELATED_LABEL" "$SELECTED_RGB" "$UNRELATED_RGB" \
  "$UNRELATED_PLACEMENT" "$SELECTED_PID" "$UNRELATED_PID" "$SELECTED_COMMAND_FILE" "$SELECTED_ACK_FILE" \
  "$SELECTED_EVENT_LOG" "$UNRELATED_EVENT_LOG" "$SELECTED_BUNDLE_ID" "$UNRELATED_BUNDLE_ID" \
  "$SELECTED_SECONDARY_RGB" "$SELECTED_AUDIO_FREQUENCY_HZ" "$UNRELATED_AUDIO_FREQUENCY_HZ" <<'PY'
import json
import shlex
import sys

(
    env_path,
    manifest_path,
    cleanup_path,
    control_path,
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
    selected_command_file,
    selected_ack_file,
    selected_event_log,
    unrelated_event_log,
    selected_bundle_id,
    unrelated_bundle_id,
    selected_secondary_rgb,
    selected_audio_frequency_hz,
    unrelated_audio_frequency_hz,
) = sys.argv[1:24]

exports = {
    "EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB": selected_rgb,
    "EASYNET_REMOTEAPP_SELECTED_SECONDARY_SENTINEL_RGB": selected_secondary_rgb,
    "EASYNET_REMOTEAPP_SELECTED_SECONDARY_SENTINEL_LABEL": (
        f"{selected_label} secondary" if selected_secondary_rgb else ""
    ),
    "EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB": unrelated_rgb,
    "EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL": selected_label,
    "EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL": unrelated_label,
    "EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PLACEMENT": unrelated_placement,
    "EASYNET_REMOTEAPP_TARGET_HINT": selected_label,
    "EASYNET_REMOTEAPP_TARGET_PID": selected_pid,
    "EASYNET_REMOTEAPP_SELECTED_SENTINEL_PID": selected_pid,
    "EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PID": unrelated_pid,
    "EASYNET_REMOTEAPP_SENTINEL_FIXTURE_MANIFEST": manifest_path,
    "EASYNET_REMOTEAPP_SELECTED_CONTROL_SH": control_path,
    "EASYNET_REMOTEAPP_SELECTED_INPUT_EVENT_LOG": selected_event_log,
    "EASYNET_REMOTEAPP_UNRELATED_INPUT_EVENT_LOG": unrelated_event_log,
    "EASYNET_REMOTEAPP_SELECTED_SENTINEL_BUNDLE_ID": selected_bundle_id,
    "EASYNET_REMOTEAPP_UNRELATED_SENTINEL_BUNDLE_ID": unrelated_bundle_id,
}
if selected_audio_frequency_hz:
    exports["EASYNET_REMOTEAPP_EXPECTED_AUDIO_FREQUENCY_HZ"] = selected_audio_frequency_hz
    exports["EASYNET_REMOTEAPP_UNRELATED_AUDIO_FREQUENCY_HZ"] = unrelated_audio_frequency_hz

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
        "bundle_id": selected_bundle_id or None,
        "input_event_log": selected_event_log,
        "audio_tone_frequency_hz": (
            float(selected_audio_frequency_hz) if selected_audio_frequency_hz else None
        ),
        "surfaces": [
            {
                "role": "primary",
                "label": selected_label,
                "rgb": [int(part) for part in selected_rgb.split(",")],
            },
            *(
                [{
                    "role": "secondary",
                    "label": f"{selected_label} secondary",
                    "rgb": [int(part) for part in selected_secondary_rgb.split(",")],
                }]
                if selected_secondary_rgb
                else []
            ),
        ],
    },
    "unrelated": {
        "label": unrelated_label,
        "rgb": [int(part) for part in unrelated_rgb.split(",")],
        "pid": int(unrelated_pid),
        "bundle_id": unrelated_bundle_id or None,
        "placement": unrelated_placement,
        "input_event_log": unrelated_event_log,
        "audio_tone_frequency_hz": (
            float(unrelated_audio_frequency_hz) if unrelated_audio_frequency_hz else None
        ),
    },
}
with open(manifest_path, "w", encoding="utf-8") as f:
    json.dump(manifest, f, indent=2, sort_keys=True)
    f.write("\n")

with open(control_path, "w", encoding="utf-8") as f:
    f.write("#!/usr/bin/env bash\n")
    f.write("set -euo pipefail\n")
    f.write("ACTION=\"${1:?action required}\"\n")
    f.write(f"COMMAND_FILE={shlex.quote(selected_command_file)}\n")
    f.write(f"ACK_FILE={shlex.quote(selected_ack_file)}\n")
    f.write("wait_ack() {\n")
    f.write("  local expected=\"${1:?expected ack required}\"\n")
    f.write("  python3 - \"$ACK_FILE\" \"$expected\" <<'ACKPY'\n")
    f.write("import pathlib, sys, time\n")
    f.write("ack = pathlib.Path(sys.argv[1])\n")
    f.write("expected = sys.argv[2]\n")
    f.write("deadline = time.time() + 5.0\n")
    f.write("while time.time() < deadline:\n")
    f.write("    if ack.exists() and ack.read_text(encoding='utf-8').strip() == expected:\n")
    f.write("        raise SystemExit(0)\n")
    f.write("    time.sleep(0.05)\n")
    f.write("raise SystemExit(f'sentinel action {expected} was not acknowledged by AppKit fixture')\n")
    f.write("ACKPY\n")
    f.write("}\n")
    f.write("send_move_resize() {\n")
    f.write("  local x=\"${1:?x required}\" y=\"${2:?y required}\" width=\"${3:?width required}\" height=\"${4:?height required}\"\n")
    f.write("  rm -f \"$ACK_FILE\"\n")
    f.write("  printf 'move_resize %s %s %s %s\\n' \"$x\" \"$y\" \"$width\" \"$height\" >\"$COMMAND_FILE\"\n")
    f.write("  wait_ack move_resize\n")
    f.write("}\n")
    f.write("send_move() {\n")
    f.write("  local x=\"${1:?x required}\" y=\"${2:?y required}\"\n")
    f.write("  rm -f \"$ACK_FILE\"\n")
    f.write("  printf 'move %s %s\\n' \"$x\" \"$y\" >\"$COMMAND_FILE\"\n")
    f.write("  wait_ack move\n")
    f.write("}\n")
    f.write("case \"$ACTION\" in\n")
    f.write("  focus)\n")
    f.write("    rm -f \"$ACK_FILE\"\n")
    f.write("    printf 'focus %s-%s\\n' \"$$\" \"$RANDOM\" >\"$COMMAND_FILE\"\n")
    f.write("    wait_ack focus\n")
    f.write("    ;;\n")
    f.write("  move-resize)\n")
    f.write("    send_move 120 220\n")
    f.write("    sleep 1.1\n")
    f.write("    send_move_resize 120 220 620 360\n")
    f.write("    ;;\n")
    f.write("  close)\n")
    f.write("    rm -f \"$ACK_FILE\"\n")
    f.write("    printf 'close\\n' >\"$COMMAND_FILE\"\n")
    f.write("    wait_ack close\n")
    f.write("    ;;\n")
    f.write("  *) echo \"unknown selected sentinel action: $ACTION\" >&2; exit 64 ;;\n")
    f.write("esac\n")

with open(cleanup_path, "w", encoding="utf-8") as f:
    f.write("#!/usr/bin/env bash\n")
    f.write("set -euo pipefail\n")
    f.write(f"{shlex.quote(repo_root + '/tools/scripts/host-remoteapp-sentinel-fixture.sh')} --stop --out-dir {shlex.quote(out_dir)}\n")
PY
chmod +x "$CLEANUP_SH"
chmod +x "$SELECTED_CONTROL_SH"
FIXTURE_HANDOFF_COMPLETE=1

printf 'host-remoteapp-sentinel-fixture: started selected_pid=%s unrelated_pid=%s env=%s\n' \
  "$SELECTED_PID" "$UNRELATED_PID" "$ENV_FILE"
