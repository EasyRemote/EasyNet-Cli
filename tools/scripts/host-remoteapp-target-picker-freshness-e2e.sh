#!/usr/bin/env bash
# Host-side remoteapp target picker freshness E2E.
#
# Boundary:
# - This script proves SPEC E2E-01 at the daemon/frontend contract boundary:
#   a known native window/application is opened after daemon boot, the picker inventory is
#   refreshed through resource.refresh_remote_targets, and the selected row is
#   live, available, fresh, and addressable by Resource URA.
# - It does not create a remote desktop session or validate media. Session
#   binding and decoded-frame isolation are owned by the other remoteapp E2E
#   harnesses.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
PROVENANCE_HELPER="$SELF_DIR/remoteapp-evidence-provenance.py"
BUNDLED_SENTINEL_FIXTURE="$REPO_ROOT/tools/scripts/host-remoteapp-sentinel-fixture.sh"

MODE=run
TARGET_KIND=window
OUT_DIR=""
SENTINEL_FIXTURE=0
SENTINEL_FIXTURE_CMD="${EASYNET_REMOTEAPP_SENTINEL_FIXTURE_CMD:-}"

usage() {
  cat <<'USAGE'
Usage:
  host-remoteapp-target-picker-freshness-e2e.sh --run --sentinel-fixture
  host-remoteapp-target-picker-freshness-e2e.sh --self-test

Options:
  --run                 Execute against the local EasyNet daemon.
  --self-test           Validate the harness against synthetic positive evidence.
  --target-kind KIND    window or application. Default: window.
  --sentinel-fixture    Launch the bundled native AppKit selected/unrelated
                        fixture and select the known window/application.
  --sentinel-fixture-cmd CMD
                        Override fixture command. Receives
                        EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR and must write
                        env.sh plus cleanup.sh.
  --out-dir DIR         Report directory. Defaults under target/e2e.

Environment:
  EASYNET_REMOTEAPP_EASYNET_BIN
                        Optional easynet binary override.
  EASYNET_REMOTEAPP_SENTINEL_FIXTURE_CMD
                        Same as --sentinel-fixture-cmd.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) MODE=self-test; shift ;;
    --target-kind)
      case "${2:?missing value for --target-kind}" in
        window|application) TARGET_KIND="$2" ;;
        *) echo "invalid target picker freshness kind: $2" >&2; exit 64 ;;
      esac
      shift 2
      ;;
    --sentinel-fixture) SENTINEL_FIXTURE=1; shift ;;
    --sentinel-fixture-cmd)
      SENTINEL_FIXTURE=1
      SENTINEL_FIXTURE_CMD="${2:?missing value for --sentinel-fixture-cmd}"
      shift 2
      ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

TIMESTAMP="$(date -u +%Y%m%d-%H%M%S)"
if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$REPO_ROOT/target/e2e/host-remoteapp-target-picker-freshness/$TIMESTAMP-$TARGET_KIND-$$"
fi
mkdir -p "$OUT_DIR"

EVIDENCE_JSON="$OUT_DIR/evidence.json"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"
RUNTIME_STATUS_JSON="$OUT_DIR/runtime-status-before-fixture.json"
LIVE_INVENTORY_JSON="$OUT_DIR/live-inventory.json"
SELECTED_RESOURCE_JSON="$OUT_DIR/selected-resource.json"
SENTINEL_MANIFEST_JSON="$OUT_DIR/sentinel-manifest.json"

die() {
  echo "[FAIL] $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}

run_easynet() {
  if [[ -n "${EASYNET_REMOTEAPP_EASYNET_BIN:-}" ]]; then
    "$EASYNET_REMOTEAPP_EASYNET_BIN" "$@"
  elif [[ -x "$REPO_ROOT/target/debug/easynet" ]]; then
    "$REPO_ROOT/target/debug/easynet" "$@"
  else
    need_cmd cargo
    cargo run --quiet --bin easynet -- "$@"
  fi
}

unix_ms_now() {
  python3 - <<'PY'
import time
print(int(time.time() * 1000))
PY
}

write_failure_report() {
  local reason="$1"
  python3 - "$REPORT_JSON" "$REPORT_MD" "$reason" <<'PY'
import json
import pathlib
import sys

report_path, md_path, reason = sys.argv[1:4]
report = {
    "script": "tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh",
    "status": "failed",
    "reason": reason,
    "phase": "daemon_invocation_preflight",
    "evidence_json": None,
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(
    json.dumps(report, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
with open(md_path, "w", encoding="utf-8") as f:
    f.write("# RemoteApp target picker freshness E2E report\n\n")
    f.write("- Status: `failed`\n")
    f.write("- Phase: `daemon_invocation_preflight`\n")
    f.write(f"- Reason: `{reason}`\n")
PY
}

preflight_daemon_invocation_ready() {
  run_easynet runtime status --json >"$RUNTIME_STATUS_JSON"
  python3 - "$RUNTIME_STATUS_JSON" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as f:
    status = json.load(f)

errors = []
daemon = status.get("daemon") if isinstance(status.get("daemon"), dict) else {}
runtime = status.get("runtime") if isinstance(status.get("runtime"), dict) else {}

if daemon.get("pid_alive") is not True:
    errors.append("daemon.pid_alive is not true")
if daemon.get("invocation_accepting") is not True:
    errors.append("daemon.invocation_accepting is not true")
started = runtime.get("started_at")
if not isinstance(started, str) or not started:
    errors.append("runtime.started_at is missing")

if errors:
    raise SystemExit("; ".join(errors))
PY
}

validate_evidence() {
  python3 "$PROVENANCE_HELPER" verify --mode "$MODE" --evidence "$EVIDENCE_JSON"
  python3 - "$EVIDENCE_JSON" "$REPORT_JSON" "$REPORT_MD" <<'PY'
import json
import pathlib
import sys

evidence_path, report_path, md_path = sys.argv[1:4]
with open(evidence_path, encoding="utf-8") as f:
    evidence = json.load(f)

errors = []

def require(condition, message):
    if not condition:
        errors.append(message)

def get(path, default=None):
    value = evidence
    for part in path.split("."):
        if not isinstance(value, dict) or part not in value:
            return default
        value = value[part]
    return value

def int_value(value):
    return value if isinstance(value, int) and not isinstance(value, bool) else None

selected = evidence.get("selected_resource")
inventory = evidence.get("live_inventory")
fixture = evidence.get("sentinel_fixture")
runtime = evidence.get("runtime_before_fixture")
timing = evidence.get("timing")

require(evidence.get("status") == "passed", "evidence.status must be passed")
target_kind = evidence.get("target_kind")
require(target_kind in {"window", "application"},
        "target_kind must be window or application")
require(isinstance(runtime, dict), "runtime_before_fixture must be recorded")
require(isinstance(fixture, dict), "sentinel_fixture must be recorded")
require(isinstance(inventory, dict), "live_inventory must be recorded")
require(isinstance(selected, dict), "selected_resource must be recorded")
require(isinstance(timing, dict), "timing must be recorded")

runtime_started_at_ms = int_value(get("runtime_before_fixture.started_at_ms"))
fixture_launch_started_at_ms = int_value(get("timing.fixture_launch_started_at_ms"))
fixture_ready_at_ms = int_value(get("timing.fixture_ready_at_ms"))
refresh_started_at_ms = int_value(get("timing.refresh_started_at_ms"))
refresh_completed_at_ms = int_value(get("timing.refresh_completed_at_ms"))
inventory_observed_at_ms = int_value(get("live_inventory.observed_at_ms"))

require(runtime_started_at_ms is not None, "runtime started_at_ms must be recorded")
require(fixture_launch_started_at_ms is not None, "fixture_launch_started_at_ms must be recorded")
require(fixture_ready_at_ms is not None, "fixture_ready_at_ms must be recorded")
require(refresh_started_at_ms is not None, "refresh_started_at_ms must be recorded")
require(refresh_completed_at_ms is not None, "refresh_completed_at_ms must be recorded")
require(inventory_observed_at_ms is not None, "live inventory observed_at_ms must be recorded")
if runtime_started_at_ms is not None and fixture_launch_started_at_ms is not None:
    require(runtime_started_at_ms <= fixture_launch_started_at_ms,
            "known target window must be opened after daemon boot")
if fixture_ready_at_ms is not None and refresh_started_at_ms is not None:
    require(fixture_ready_at_ms <= refresh_started_at_ms,
            "live inventory refresh must run after the known window fixture is ready")
if refresh_started_at_ms is not None and refresh_completed_at_ms is not None and inventory_observed_at_ms is not None:
    require(refresh_started_at_ms <= inventory_observed_at_ms <= refresh_completed_at_ms,
            "live inventory observed_at_ms must fall within the refresh call window")

require(get("live_inventory.ability") == "resource.refresh_remote_targets",
        "live inventory must use resource.refresh_remote_targets")
require(get("live_inventory.target_kind") == target_kind,
        "live inventory target_kind must match the requested target kind")
require(int_value(get("live_inventory.freshness_ttl_ms")) is not None,
        "live inventory must report freshness_ttl_ms")
owner_pid_field = "pid" if target_kind == "window" else "primary_pid"
selected_owner_pid = get(f"selected_resource.metadata.{owner_pid_field}")
require(get("sentinel_fixture.selected.pid") == selected_owner_pid,
        f"selected resource metadata.{owner_pid_field} must match selected sentinel pid")
require(get("sentinel_fixture.unrelated.pid") != selected_owner_pid,
        f"selected resource metadata.{owner_pid_field} must not match the unrelated sentinel pid")
require(get("selection_identity.owner_pid") == get("sentinel_fixture.selected.pid"),
        "selection identity owner_pid must match the selected sentinel pid")
if target_kind == "window":
    require(get("selection_identity.kind") == "window_id_plus_owner_pid",
            "window selection identity must use window_id_plus_owner_pid")
    window_id = int_value(get("selected_resource.metadata.window_id"))
    require(window_id is not None and window_id > 0,
            "selected window metadata.window_id must be a positive native identifier")
    require(get("selection_identity.window_id") == window_id,
            "window selection identity must bind metadata.window_id")
else:
    require(get("selection_identity.kind") == "application_identity_plus_owner_pid_and_window_set",
            "application selection identity must bind application identity, owner pid, and window set")
    app_identity = get("selected_resource.metadata.app_identity")
    bundle_id = get("selected_resource.metadata.bundle_id")
    require(isinstance(app_identity, str) and app_identity,
            "selected application metadata.app_identity must be non-empty")
    require(app_identity == bundle_id == get("sentinel_fixture.selected.bundle_id"),
            "selected application identity must match the sentinel bundle id")
    require(get("selection_identity.app_identity") == app_identity,
            "application selection identity must bind metadata.app_identity")
    window_ids = get("selected_resource.metadata.resolved_window_ids")
    valid_window_ids = window_ids if isinstance(window_ids, list) else []
    require(len(valid_window_ids) >= 2
            and all(int_value(item) is not None and item > 0 for item in valid_window_ids),
            "selected application must expose at least two positive resolved_window_ids")
    require(get("selected_resource.metadata.window_count") == len(valid_window_ids),
            "selected application window_count must match resolved_window_ids")
    require(get("selection_identity.resolved_window_ids") == valid_window_ids,
            "application selection identity must bind the exact resolved_window_ids")
    surface_ids = [
        surface.get("window_id")
        for surface in (get("selected_resource.metadata.front_to_back_surfaces") or [])
        if isinstance(surface, dict) and int_value(surface.get("window_id")) is not None
    ]
    require(len(surface_ids) == len(valid_window_ids)
            and sorted(surface_ids) == sorted(valid_window_ids),
            "application front_to_back_surfaces must bind the resolved window set")
    require(get("selected_resource.metadata.display_scoped") is False,
            "application selection must remain process-scoped rather than display-scoped")
    window_set_epoch = int_value(get("selected_resource.metadata.window_set_epoch"))
    require(window_set_epoch is not None and window_set_epoch > 0,
            "application window_set_epoch must be a positive JSON-safe identity")
    require(get("selection_identity.window_set_epoch") == window_set_epoch,
            "application selection identity must bind metadata.window_set_epoch")
require(get("selected_resource.type") == target_kind,
        "selected resource type must match the requested target kind")
resource_ura = get("selected_resource.resource_ura")
require(isinstance(resource_ura, str) and resource_ura.startswith("easynet:///"),
        "selected resource_ura must be a canonical EasyNet URA")
require(get("selected_resource.metadata.availability") == "available",
        "selected resource availability must be available")
require(get("selected_resource.metadata.discovery_source") == "resource.refresh_remote_targets",
        "selected resource must be discovered by resource.refresh_remote_targets")
require(get("selected_resource.metadata.inventory_source") == "daemon_resource_inventory",
        "selected resource must come from daemon resource inventory")
freshness = get("selected_resource.metadata.freshness")
require(isinstance(freshness, dict), "selected resource metadata.freshness must be an object")
if isinstance(freshness, dict):
    require(freshness.get("source") == "live_refresh",
            "selected resource freshness.source must be live_refresh")
    observed = int_value(freshness.get("observed_at_ms"))
    stale_after = int_value(freshness.get("stale_after_ms"))
    require(observed is not None, "selected resource freshness.observed_at_ms must be an integer")
    require(stale_after is not None, "selected resource freshness.stale_after_ms must be an integer")
    if observed is not None and stale_after is not None:
        require(stale_after > observed,
                "selected resource freshness.stale_after_ms must be after observed_at_ms")
    if inventory_observed_at_ms is not None and observed is not None:
        require(observed == inventory_observed_at_ms,
                "selected resource freshness.observed_at_ms must match live inventory observed_at_ms")

require(evidence.get("selection_state") == "selected_from_live_refresh",
        "selection_state must prove the picker selected from the live refresh result")

report = {
    "script": "tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh",
    "status": "failed" if errors else "passed",
    "errors": errors,
    "evidence_json": evidence_path,
    "product_complete_claim": False,
    "target_kind": target_kind,
    "selected_resource_ura": resource_ura,
    "selection_identity_kind": get("selection_identity.kind"),
    "inventory_observed_at_ms": inventory_observed_at_ms,
}
pathlib.Path(report_path).write_text(
    json.dumps(report, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
with open(md_path, "w", encoding="utf-8") as f:
    f.write("# RemoteApp target picker freshness E2E report\n\n")
    f.write(f"- Status: `{report['status']}`\n")
    f.write(f"- Target kind: `{report['target_kind']}`\n")
    f.write(f"- Evidence: `{evidence_path}`\n")
    f.write(f"- Selected Resource URA: `{report['selected_resource_ura']}`\n")
    f.write(f"- Inventory observed_at_ms: `{report['inventory_observed_at_ms']}`\n")
    if errors:
        f.write("\n## Errors\n")
        for error in errors:
            f.write(f"- {error}\n")
if errors:
    for error in errors:
        print(error, file=sys.stderr)
    raise SystemExit(1)
PY
  python3 "$PROVENANCE_HELPER" project-report --mode "$MODE" \
    --evidence "$EVIDENCE_JSON" --report "$REPORT_JSON"
}

if [[ "$MODE" == "self-test" ]]; then
  python3 - "$EVIDENCE_JSON" "$TARGET_KIND" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
target_kind = sys.argv[2]
resource_ura = f"easynet:///r/localhost/resource/device.dev/streams/{target_kind}.fresh"
runtime_started = 1787000000000
fixture_started = runtime_started + 1000
fixture_ready = fixture_started + 2000
refresh_started = fixture_ready + 100
observed = refresh_started + 10
refresh_done = observed + 80
selected_pid = 4242
selected_label = f"EasyNet selected {target_kind} sentinel fixture"
bundle_id = "tech.silan.easynet.remoteapp.selectedsentinel.selftest"
if target_kind == "window":
    selected_metadata = {
        "pid": selected_pid,
        "window_id": 7001,
        "title": selected_label,
    }
    selection_identity = {
        "kind": "window_id_plus_owner_pid",
        "owner_pid": selected_pid,
        "window_id": 7001,
        "diagnostic_label_observable": True,
    }
    selected_fixture = {
        "label": selected_label,
        "pid": selected_pid,
    }
else:
    selected_metadata = {
        "primary_pid": selected_pid,
        "app_identity": bundle_id,
        "bundle_id": bundle_id,
        "resolved_window_ids": [7001, 7002],
        "window_count": 2,
        "front_to_back_surfaces": [
            {"window_id": 7002, "x": 100, "y": 100, "width": 640, "height": 480},
            {"window_id": 7001, "x": 0, "y": 0, "width": 640, "height": 480},
        ],
        "display_scoped": False,
        "window_set_epoch": 101,
    }
    selection_identity = {
        "kind": "application_identity_plus_owner_pid_and_window_set",
        "owner_pid": selected_pid,
        "app_identity": bundle_id,
        "resolved_window_ids": [7001, 7002],
        "window_set_epoch": 101,
        "diagnostic_label_observable": True,
    }
    selected_fixture = {
        "label": selected_label,
        "pid": selected_pid,
        "bundle_id": bundle_id,
        "surfaces": [
            {"label": selected_label, "role": "primary"},
            {"label": f"{selected_label} secondary", "role": "secondary"},
        ],
    }
selected_metadata.update({
    "availability": "available",
    "discovery_source": "resource.refresh_remote_targets",
    "inventory_source": "daemon_resource_inventory",
    "freshness": {
        "observed_at_ms": observed,
        "stale_after_ms": observed + 5000,
        "source": "live_refresh",
    },
})
evidence = {
    "status": "passed",
    "evidence_origin": "contract_self_test",
    "target_kind": target_kind,
    "selection_state": "selected_from_live_refresh",
    "runtime_before_fixture": {
        "state": "FRONTEND_CONNECTED",
        "started_at_ms": runtime_started,
    },
    "timing": {
        "fixture_launch_started_at_ms": fixture_started,
        "fixture_ready_at_ms": fixture_ready,
        "refresh_started_at_ms": refresh_started,
        "refresh_completed_at_ms": refresh_done,
    },
    "sentinel_fixture": {
        "target_kind": target_kind,
        "selected": selected_fixture,
        "unrelated": {
            "label": f"EasyNet unrelated {target_kind} sentinel fixture",
            "pid": selected_pid + 1,
        },
    },
    "live_inventory": {
        "ability": "resource.refresh_remote_targets",
        "target_kind": target_kind,
        "observed_at_ms": observed,
        "freshness_ttl_ms": 5000,
        "resource_count": 1,
    },
    "selected_resource": {
        "resource_ura": resource_ura,
        "type": target_kind,
        "display_name": selected_label,
        "metadata": selected_metadata,
    },
    "selection_identity": selection_identity,
}
path.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
  validate_evidence
  echo "host-remoteapp-target-picker-freshness-e2e self-test ok"
  exit 0
fi

[[ "$TARGET_KIND" == "window" || "$TARGET_KIND" == "application" ]] || \
  die "unsupported target kind: $TARGET_KIND"
[[ "$SENTINEL_FIXTURE" == "1" ]] || die "--sentinel-fixture is required for live target picker freshness E2E"
if [[ -z "$SENTINEL_FIXTURE_CMD" ]]; then
  [[ -x "$BUNDLED_SENTINEL_FIXTURE" ]] || die "missing bundled sentinel fixture: $BUNDLED_SENTINEL_FIXTURE"
  SENTINEL_FIXTURE_CMD="$BUNDLED_SENTINEL_FIXTURE --target-kind $TARGET_KIND"
fi

need_cmd python3

if ! preflight_output="$(preflight_daemon_invocation_ready 2>&1)"; then
  write_failure_report "daemon_invocation_preflight_failed: $preflight_output"
  die "daemon invocation preflight failed before launching sentinel fixture: $preflight_output"
fi
RUNTIME_STARTED_AT_MS="$(python3 - "$RUNTIME_STATUS_JSON" <<'PY'
import datetime
import json
import sys

with open(sys.argv[1], encoding="utf-8") as f:
    status = json.load(f)
started = status.get("runtime", {}).get("started_at")
if not isinstance(started, str) or not started:
    raise SystemExit("runtime status did not report runtime.started_at")
if started.endswith("Z"):
    started = started[:-1] + "+00:00"
dt = datetime.datetime.fromisoformat(started)
if dt.tzinfo is None:
    dt = dt.replace(tzinfo=datetime.timezone.utc)
print(int(dt.timestamp() * 1000))
PY
)"

SENTINEL_FIXTURE_DIR="$OUT_DIR/sentinel-fixture"
mkdir -p "$SENTINEL_FIXTURE_DIR"
export EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR="$SENTINEL_FIXTURE_DIR"
trap '[[ -x "$SENTINEL_FIXTURE_DIR/cleanup.sh" ]] && "$SENTINEL_FIXTURE_DIR/cleanup.sh" >/dev/null 2>&1 || true' EXIT
FIXTURE_LAUNCH_STARTED_AT_MS="$(unix_ms_now)"
bash -lc "$SENTINEL_FIXTURE_CMD"
FIXTURE_READY_AT_MS="$(unix_ms_now)"
[[ -f "$SENTINEL_FIXTURE_DIR/env.sh" ]] || die "sentinel fixture did not write env.sh"
source "$SENTINEL_FIXTURE_DIR/env.sh"
[[ -n "${EASYNET_REMOTEAPP_TARGET_PID:-}" ]] || die "sentinel fixture did not export selected target pid"
[[ -n "${EASYNET_REMOTEAPP_TARGET_HINT:-}" ]] || die "sentinel fixture did not export selected target hint"
[[ -f "${EASYNET_REMOTEAPP_SENTINEL_FIXTURE_MANIFEST:-}" ]] || die "sentinel fixture did not export manifest path"
cp "$EASYNET_REMOTEAPP_SENTINEL_FIXTURE_MANIFEST" "$SENTINEL_MANIFEST_JSON"

REFRESH_STARTED_AT_MS="$(unix_ms_now)"
run_easynet ability refresh-remote-targets --type "$TARGET_KIND" --format json >"$LIVE_INVENTORY_JSON"
REFRESH_COMPLETED_AT_MS="$(unix_ms_now)"

python3 "$SELF_DIR/remoteapp-select-live-target.py" \
  --inventory "$LIVE_INVENTORY_JSON" \
  --output "$SELECTED_RESOURCE_JSON" \
  --kind "$TARGET_KIND" \
  --pid "$EASYNET_REMOTEAPP_TARGET_PID" \
  --hint "$EASYNET_REMOTEAPP_TARGET_HINT"

python3 - "$EVIDENCE_JSON" "$TARGET_KIND" "$RUNTIME_STATUS_JSON" "$SENTINEL_MANIFEST_JSON" "$LIVE_INVENTORY_JSON" "$SELECTED_RESOURCE_JSON" \
  "$RUNTIME_STARTED_AT_MS" "$FIXTURE_LAUNCH_STARTED_AT_MS" "$FIXTURE_READY_AT_MS" "$REFRESH_STARTED_AT_MS" "$REFRESH_COMPLETED_AT_MS" <<'PY'
import json
import pathlib
import sys

(
    evidence_path,
    target_kind,
    runtime_status_path,
    fixture_manifest_path,
    live_inventory_path,
    selected_resource_path,
    runtime_started_at_ms,
    fixture_launch_started_at_ms,
    fixture_ready_at_ms,
    refresh_started_at_ms,
    refresh_completed_at_ms,
) = sys.argv[1:12]

with open(runtime_status_path, encoding="utf-8") as f:
    runtime_status = json.load(f)
with open(fixture_manifest_path, encoding="utf-8") as f:
    fixture = json.load(f)
with open(live_inventory_path, encoding="utf-8") as f:
    live_inventory = json.load(f)
with open(selected_resource_path, encoding="utf-8") as f:
    selected = json.load(f)

selected_metadata = selected.get("metadata") if isinstance(selected.get("metadata"), dict) else {}
selected_label = fixture.get("selected", {}).get("label")
diagnostic_values = (
    selected.get("display_name"),
    selected_metadata.get("title"),
    selected_metadata.get("app_name"),
)
if target_kind == "window":
    selection_identity = {
        "kind": "window_id_plus_owner_pid",
        "owner_pid": selected_metadata.get("pid"),
        "window_id": selected_metadata.get("window_id"),
        "diagnostic_label_observable": any(
            value == selected_label for value in diagnostic_values if value is not None
        ),
    }
else:
    selection_identity = {
        "kind": "application_identity_plus_owner_pid_and_window_set",
        "owner_pid": selected_metadata.get("primary_pid"),
        "app_identity": selected_metadata.get("app_identity"),
        "resolved_window_ids": selected_metadata.get("resolved_window_ids"),
        "window_set_epoch": selected_metadata.get("window_set_epoch"),
        "diagnostic_label_observable": any(
            value == selected_label for value in diagnostic_values if value is not None
        ),
    }

evidence = {
    "status": "passed",
    "evidence_origin": "live_runner",
    "target_kind": target_kind,
    "selection_state": "selected_from_live_refresh",
    "runtime_before_fixture": {
        "state": runtime_status.get("connection", {}).get("state"),
        "device_ura": runtime_status.get("connection", {}).get("device_ura"),
        "started_at": runtime_status.get("runtime", {}).get("started_at"),
        "started_at_ms": int(runtime_started_at_ms),
    },
    "timing": {
        "fixture_launch_started_at_ms": int(fixture_launch_started_at_ms),
        "fixture_ready_at_ms": int(fixture_ready_at_ms),
        "refresh_started_at_ms": int(refresh_started_at_ms),
        "refresh_completed_at_ms": int(refresh_completed_at_ms),
    },
    "sentinel_fixture": fixture,
    "live_inventory": {
        "ability": "resource.refresh_remote_targets",
        "target_kind": target_kind,
        "observed_at_ms": live_inventory.get("observed_at_ms"),
        "freshness_ttl_ms": live_inventory.get("freshness_ttl_ms"),
        "retired_count": live_inventory.get("retired_count"),
        "screen_target_discovery_available": live_inventory.get("screen_target_discovery_available"),
        "resource_count": len(live_inventory.get("resources", []))
        if isinstance(live_inventory.get("resources"), list)
        else None,
    },
    "selected_resource": selected,
    "selection_identity": selection_identity,
}
pathlib.Path(evidence_path).write_text(
    json.dumps(evidence, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY

validate_evidence
echo "host-remoteapp-target-picker-freshness-e2e ok: $REPORT_MD"
