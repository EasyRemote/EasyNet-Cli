#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SELECTOR="$ROOT/tools/scripts/remoteapp-select-live-target.py"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

python3 - "$TMP/inventory.json" <<'PY'
import json, pathlib, sys

def window(ura, pid, window_id, display_name, title=None):
    return {
        "resource_ura": ura,
        "type": "window",
        "display_name": display_name,
        "metadata": {
            "availability": "available",
            "pid": pid,
            "window_id": window_id,
            "title": title,
        },
    }

inventory = {
    "resources": [
        window("easynet:///r/test/resource/device.host/streams/window.selected", 4242, 7001,
               "easynet-remoteapp-selected-sent", None),
        window("easynet:///r/test/resource/device.host/streams/window.unrelated", 4243, 7002,
               "easynet-remoteapp-unrelated-sen", None),
        {
            "resource_ura": "easynet:///r/test/resource/device.host/streams/application.editor",
            "type": "application",
            "display_name": "Editor",
            "metadata": {
                "availability": "available",
                "primary_pid": 5000,
                "resolved_window_ids": [8001, 8002],
            },
        },
    ]
}
pathlib.Path(sys.argv[1]).write_text(json.dumps(inventory), encoding="utf-8")
PY

# macOS may redact the title and truncate the process display name.  The host
# fixture PID still selects the window, whose native window_id binds the session.
python3 "$SELECTOR" --inventory "$TMP/inventory.json" --output "$TMP/selected.json" \
  --kind window --pid 4242 --hint 'EasyNet selected window sentinel full label'
jq -e '.metadata.pid == 4242 and .metadata.window_id == 7001' "$TMP/selected.json" >/dev/null

python3 "$SELECTOR" --inventory "$TMP/inventory.json" --output "$TMP/application.json" \
  --kind application --pid 5000
jq -e '.metadata.resolved_window_ids == [8001, 8002]' "$TMP/application.json" >/dev/null

expect_failure() {
  local label="$1"
  shift
  if "$@" >"$TMP/$label.stdout" 2>"$TMP/$label.stderr"; then
    echo "[FAIL] expected selector failure: $label" >&2
    exit 1
  fi
}

python3 - "$TMP/inventory.json" "$TMP/ambiguous.json" <<'PY'
import json, pathlib, sys
value = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
duplicate = dict(value["resources"][0])
duplicate["resource_ura"] = "easynet:///r/test/resource/device.host/streams/window.second"
duplicate["metadata"] = dict(duplicate["metadata"], window_id=7003)
value["resources"].append(duplicate)
pathlib.Path(sys.argv[2]).write_text(json.dumps(value), encoding="utf-8")
PY
expect_failure ambiguous_pid python3 "$SELECTOR" --inventory "$TMP/ambiguous.json" \
  --output "$TMP/no.json" --kind window --pid 4242

python3 - "$TMP/inventory.json" "$TMP/missing-native.json" <<'PY'
import json, pathlib, sys
value = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
value["resources"][0]["metadata"].pop("window_id")
pathlib.Path(sys.argv[2]).write_text(json.dumps(value), encoding="utf-8")
PY
expect_failure missing_native python3 "$SELECTOR" --inventory "$TMP/missing-native.json" \
  --output "$TMP/no.json" --kind window --pid 4242

expect_failure missing_ura python3 "$SELECTOR" --inventory "$TMP/inventory.json" \
  --output "$TMP/no.json" --kind window \
  --resource-ura easynet:///r/test/resource/device.host/streams/window.missing

echo "remoteapp-select-live-target tests passed"
