#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-e2e-acceptance-boundary.sh"
HARNESS="$REPO_ROOT/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
PROBE_HARNESS="$REPO_ROOT/tools/scripts/host-remoteapp-decoded-frame-probe.sh"
SENTINEL_FIXTURE="$REPO_ROOT/tools/scripts/host-remoteapp-sentinel-fixture.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

mkdir -p "$SANDBOX/tools/scripts" "$SANDBOX/docs/design" "$SANDBOX/examples"
cp "$HARNESS" "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
cp "$PROBE_HARNESS" "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh"
cp "$SENTINEL_FIXTURE" "$SANDBOX/tools/scripts/host-remoteapp-sentinel-fixture.sh"
cp "$REPO_ROOT/examples/easynet-remoteapp-frame-receiver.rs" \
  "$SANDBOX/examples/easynet-remoteapp-frame-receiver.rs"
chmod +x "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
chmod +x "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh"
chmod +x "$SANDBOX/tools/scripts/host-remoteapp-sentinel-fixture.sh"
cat >"$SANDBOX/docs/design/remoteapp-targeted-session-spec.md" <<'MD'
| E2E-03 exact window session | decoded stream excludes sentinel |
| E2E-04 exact application session | decoded stream excludes other apps |
| E2E-07 display fallback forbidden | no decoded full display |
MD

CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

"$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  --self-test \
  --target-kind window >/dev/null
"$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  --self-test \
  --target-kind application >/dev/null

cp "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good"
perl -0pi -e 's/\$TIMESTAMP-\$TARGET_KIND-\$\$/\$TIMESTAMP/' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted colliding default report directories" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"

PROBE="$SANDBOX/fake_probe.py"
cat >"$PROBE" <<'PY'
import json
import os
import pathlib

subject = "easynet:///r/localhost/resource/device.dev/streams/window.test"
out = pathlib.Path(os.environ["EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON"]).parent
sample = out / "sample-frame.ppm"
sample.write_bytes(b"P6\n3 3\n255\n" + b"\xff\x00\x00" * 9)
evidence = {
    "status": "passed",
    "live_inventory": {"ability": "resource.refresh_remote_targets"},
    "session_id": "rd-e2e-test",
    "selected_resource_ura": subject,
    "invocation": {
        "ability": "remote_desktop.create_session",
        "subject_ura": subject,
    },
    "target_binding": {
        "target_kind": "window",
        "capture_scope": "WindowSurface",
        "binding_id": "tb_test",
        "binding_epoch": 1,
        "target_identity_epoch": 1,
        "target_geometry_revision": 1,
        "media_source_epoch": 1,
        "consent_epoch": 1,
        "subject_ura": subject,
        "resolved_identity": {
            "window_id": 7,
        },
        "scope_audit": {
            "scope_widened": False,
            "display_fallback_used": False,
        },
    },
    "sentinel_fixture": {
        "proof": "dual_target_non_leak",
        "selected": {
            "label": "selected-window-red",
            "resource_ura": subject,
            "rgb": [255, 0, 0],
            "target_kind": "window",
        },
        "unrelated": {
            "label": "unrelated-window-green",
            "placement": "other_window",
            "rgb": [0, 255, 0],
        },
    },
    "transport": {"kind": "webrtc"},
    "production_media_ready": True,
    "production_readiness": {
        "ready": True,
        "requires_production_codec": True,
        "production_codec_negotiated": True,
        "media_transport_ready": True,
        "client_media_ready": True,
    },
    "decoded_frames": {
        "count": 2,
        "rtp_packet_count": 10,
        "width": 3,
        "height": 3,
        "selected_content_present": True,
        "unrelated_sentinel_present": False,
        "full_display_leak_detected": False,
        "selected_pixel_count": 9,
        "unrelated_pixel_count": 0,
    },
    "artifacts": {
        "decoded_frame_sample": str(sample),
        "session_id": "rd-e2e-test",
        "binding_id": "tb_test",
        "binding_epoch": 1,
        "target_identity_epoch": 1,
        "target_geometry_revision": 1,
        "capture_scope": "WindowSurface",
    },
}
with open(os.environ["EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON"], "w", encoding="utf-8") as f:
    json.dump(evidence, f)
PY
EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB="255,0,0" \
EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB="0,255,0" \
EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL="selected-window-red" \
EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL="unrelated-window-green" \
"$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  --run \
  --probe-cmd "python3 '$PROBE'" \
  --out-dir "$SANDBOX/e2e-out" >/dev/null

PROBE_NO_WINDOW_ID="$SANDBOX/fake_probe_no_window_id.py"
cp "$PROBE" "$PROBE_NO_WINDOW_ID"
perl -0pi -e 's/,\n        "resolved_identity": \{\n            "window_id": 7,\n        \}//' \
  "$PROBE_NO_WINDOW_ID"
if EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB="255,0,0" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB="0,255,0" \
  EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL="selected-window-red" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL="unrelated-window-green" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
    --run \
    --probe-cmd "python3 '$PROBE_NO_WINDOW_ID'" \
    --out-dir "$SANDBOX/e2e-no-window-id" >/dev/null 2>&1; then
  echo "remoteapp e2e harness accepted window evidence without resolved_identity.window_id" >&2
  exit 1
fi

PROBE_NO_ARTIFACT_EPOCHS="$SANDBOX/fake_probe_no_artifact_epochs.py"
cp "$PROBE" "$PROBE_NO_ARTIFACT_EPOCHS"
perl -0pi -e 's/,\n        "target_identity_epoch": 1,\n        "target_geometry_revision": 1//' \
  "$PROBE_NO_ARTIFACT_EPOCHS"
if EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB="255,0,0" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB="0,255,0" \
  EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL="selected-window-red" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL="unrelated-window-green" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
    --run \
    --probe-cmd "python3 '$PROBE_NO_ARTIFACT_EPOCHS'" \
    --out-dir "$SANDBOX/e2e-no-artifact-epochs" >/dev/null 2>&1; then
  echo "remoteapp e2e harness accepted decoded artifact without target epoch/revision binding" >&2
  exit 1
fi

PROBE_NO_CLIENT_READY="$SANDBOX/fake_probe_no_client_ready.py"
cat >"$PROBE_NO_CLIENT_READY" <<'PY'
import json
import os
import pathlib

subject = "easynet:///r/localhost/resource/device.dev/streams/window.test"
out = pathlib.Path(os.environ["EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON"]).parent
sample = out / "sample-frame.ppm"
sample.write_bytes(b"P6\n3 3\n255\n" + b"\xff\x00\x00" * 9)
evidence = {
    "status": "passed",
    "live_inventory": {"ability": "resource.refresh_remote_targets"},
    "session_id": "rd-e2e-no-client-ready",
    "selected_resource_ura": subject,
    "invocation": {
        "ability": "remote_desktop.create_session",
        "subject_ura": subject,
    },
    "target_binding": {
        "target_kind": "window",
        "capture_scope": "WindowSurface",
        "binding_id": "tb_no_client_ready",
        "binding_epoch": 1,
        "target_identity_epoch": 1,
        "target_geometry_revision": 1,
        "media_source_epoch": 1,
        "consent_epoch": 1,
        "subject_ura": subject,
        "resolved_identity": {
            "window_id": 7,
        },
        "scope_audit": {
            "scope_widened": False,
            "display_fallback_used": False,
        },
    },
    "sentinel_fixture": {
        "proof": "dual_target_non_leak",
        "selected": {
            "label": "selected-window-red",
            "resource_ura": subject,
            "rgb": [255, 0, 0],
            "target_kind": "window",
        },
        "unrelated": {
            "label": "unrelated-window-green",
            "placement": "other_window",
            "rgb": [0, 255, 0],
        },
    },
    "transport": {"kind": "webrtc"},
    "production_media_ready": True,
    "production_readiness": {
        "ready": True,
        "requires_production_codec": True,
        "production_codec_negotiated": True,
        "media_transport_ready": True,
    },
    "decoded_frames": {
        "count": 2,
        "rtp_packet_count": 10,
        "width": 3,
        "height": 3,
        "selected_content_present": True,
        "unrelated_sentinel_present": False,
        "full_display_leak_detected": False,
        "selected_pixel_count": 9,
        "unrelated_pixel_count": 0,
    },
    "artifacts": {
        "decoded_frame_sample": str(sample),
        "session_id": "rd-e2e-no-client-ready",
        "binding_id": "tb_no_client_ready",
        "binding_epoch": 1,
        "target_identity_epoch": 1,
        "target_geometry_revision": 1,
        "capture_scope": "WindowSurface",
    },
}
with open(os.environ["EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON"], "w", encoding="utf-8") as f:
    json.dump(evidence, f)
PY
if EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB="255,0,0" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB="0,255,0" \
  EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL="selected-window-red" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL="unrelated-window-green" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
    --run \
    --probe-cmd "python3 '$PROBE_NO_CLIENT_READY'" \
    --out-dir "$SANDBOX/e2e-no-client-ready" >/dev/null 2>&1; then
  echo "remoteapp e2e harness accepted production readiness without client_media_ready" >&2
  exit 1
fi

FIXTURE_CMD="$SANDBOX/fake_sentinel_fixture.sh"
cat >"$FIXTURE_CMD" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

mkdir -p "$EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR"
cat >"$EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR/env.sh" <<'ENV'
export EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB='255,0,0'
export EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB='0,255,0'
export EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL='selected-window-red'
export EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL='unrelated-window-green'
export EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PLACEMENT='other_window'
export EASYNET_REMOTEAPP_TARGET_HINT='selected-window-red'
export EASYNET_REMOTEAPP_TARGET_PID='4242'
export EASYNET_REMOTEAPP_SELECTED_SENTINEL_PID='4242'
export EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PID='4243'
ENV
cat >"$EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR/cleanup.sh" <<'CLEANUP'
#!/usr/bin/env bash
set -euo pipefail
touch "$(dirname "$0")/cleanup-ran"
CLEANUP
chmod +x "$EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR/cleanup.sh"
SH
chmod +x "$FIXTURE_CMD"

PROBE_FROM_FIXTURE_ENV="$SANDBOX/fake_probe_from_fixture_env.py"
cat >"$PROBE_FROM_FIXTURE_ENV" <<'PY'
import json
import os
import pathlib

subject = "easynet:///r/localhost/resource/device.dev/streams/window.test"
out = pathlib.Path(os.environ["EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON"]).parent
sample = out / "sample-frame.ppm"
sample.write_bytes(b"P6\n3 3\n255\n" + b"\xff\x00\x00" * 9)
selected_rgb = [int(part) for part in os.environ["EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB"].split(",")]
unrelated_rgb = [int(part) for part in os.environ["EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB"].split(",")]
selected_pid = int(os.environ["EASYNET_REMOTEAPP_SELECTED_SENTINEL_PID"])
unrelated_pid = int(os.environ["EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PID"])
evidence = {
    "status": "passed",
    "live_inventory": {"ability": "resource.refresh_remote_targets"},
    "session_id": "rd-e2e-fixture-test",
    "selected_resource_ura": subject,
    "invocation": {
        "ability": "remote_desktop.create_session",
        "subject_ura": subject,
    },
    "target_binding": {
        "target_kind": "window",
        "capture_scope": "WindowSurface",
        "binding_id": "tb_fixture_test",
        "binding_epoch": 1,
        "target_identity_epoch": 1,
        "target_geometry_revision": 1,
        "media_source_epoch": 1,
        "consent_epoch": 1,
        "subject_ura": subject,
        "resolved_identity": {
            "window_id": 7,
            "pid": selected_pid,
        },
        "scope_audit": {
            "scope_widened": False,
            "display_fallback_used": False,
        },
    },
    "sentinel_fixture": {
        "proof": "dual_target_non_leak",
        "selected": {
            "label": os.environ["EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL"],
            "resource_ura": subject,
            "rgb": selected_rgb,
            "target_kind": "window",
            "pid": selected_pid,
        },
        "unrelated": {
            "label": os.environ["EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL"],
            "placement": os.environ["EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PLACEMENT"],
            "rgb": unrelated_rgb,
            "pid": unrelated_pid,
        },
    },
    "transport": {"kind": "webrtc"},
    "production_media_ready": True,
    "production_readiness": {
        "ready": True,
        "requires_production_codec": True,
        "production_codec_negotiated": True,
        "media_transport_ready": True,
        "client_media_ready": True,
    },
    "decoded_frames": {
        "count": 2,
        "rtp_packet_count": 10,
        "width": 3,
        "height": 3,
        "selected_content_present": True,
        "unrelated_sentinel_present": False,
        "full_display_leak_detected": False,
        "selected_pixel_count": 9,
        "unrelated_pixel_count": 0,
    },
    "artifacts": {
        "decoded_frame_sample": str(sample),
        "session_id": "rd-e2e-fixture-test",
        "binding_id": "tb_fixture_test",
        "binding_epoch": 1,
        "target_identity_epoch": 1,
        "target_geometry_revision": 1,
        "capture_scope": "WindowSurface",
    },
}
with open(os.environ["EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON"], "w", encoding="utf-8") as f:
    json.dump(evidence, f)
PY

"$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  --run \
  --sentinel-fixture \
  --sentinel-fixture-cmd "bash '$FIXTURE_CMD'" \
  --probe-cmd "python3 '$PROBE_FROM_FIXTURE_ENV'" \
  --out-dir "$SANDBOX/e2e-fixture-out" >/dev/null
[[ -f "$SANDBOX/e2e-fixture-out/sentinel-fixture/cleanup-ran" ]] || {
  echo "remoteapp e2e harness did not run sentinel fixture cleanup" >&2
  exit 1
}

FIXTURE_SHOULD_NOT_RUN="$SANDBOX/fixture-should-not-run.sh"
cat >"$FIXTURE_SHOULD_NOT_RUN" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
mkdir -p "$EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR"
touch "$EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR/fixture-ran-before-preflight"
cat >"$EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR/env.sh" <<'ENV'
export EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB=255,0,0
export EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB=0,255,0
export EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL=selected-window-red
export EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL=unrelated-window-green
ENV
SH
chmod +x "$FIXTURE_SHOULD_NOT_RUN"
if EASYNET_REMOTEAPP_CONTROL_DISCOVERY_JSON="$SANDBOX/missing-control.json" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
    --run \
    --sentinel-fixture \
    --sentinel-fixture-cmd "bash '$FIXTURE_SHOULD_NOT_RUN'" \
    --out-dir "$SANDBOX/e2e-preflight-out" >/dev/null 2>&1; then
  echo "remoteapp e2e harness accepted bundled probe without daemon identity preflight" >&2
  exit 1
fi
[[ ! -e "$SANDBOX/e2e-preflight-out/sentinel-fixture/fixture-ran-before-preflight" ]] || {
  echo "remoteapp e2e harness launched sentinel fixture before bundled probe runtime preflight" >&2
  exit 1
}

PROBE_APP="$SANDBOX/fake_probe_application.py"
cat >"$PROBE_APP" <<'PY'
import json
import os
import pathlib

subject = "easynet:///r/localhost/resource/device.dev/streams/application.test"
out = pathlib.Path(os.environ["EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON"]).parent
sample = out / "sample-frame.ppm"
sample.write_bytes(b"P6\n3 3\n255\n" + b"\xff\x00\x00" * 9)
evidence = {
    "status": "passed",
    "live_inventory": {"ability": "resource.refresh_remote_targets"},
    "session_id": "rd-e2e-app-test",
    "selected_resource_ura": subject,
    "invocation": {
        "ability": "remote_desktop.create_session",
        "subject_ura": subject,
    },
    "target_binding": {
        "target_kind": "application",
        "capture_scope": "AppSurface",
        "binding_id": "tb_app_test",
        "binding_epoch": 1,
        "target_identity_epoch": 99,
        "target_geometry_revision": 1,
        "media_source_epoch": 1,
        "consent_epoch": 1,
        "subject_ura": subject,
        "resolved_identity": {
            "app_identity": "com.example.SelectedApp",
            "bundle_id": "com.example.SelectedApp",
            "pid": 4242,
        },
        "app_window_set": {
            "display_id": 1,
            "bundle_id": "com.example.SelectedApp",
            "primary_pid": 4242,
            "resolved_window_ids": [70, 71],
            "window_set_epoch": 99,
        },
        "scope_audit": {
            "scope_widened": False,
            "display_fallback_used": False,
        },
    },
    "sentinel_fixture": {
        "proof": "dual_target_non_leak",
        "selected": {
            "label": "selected-app-red",
            "resource_ura": subject,
            "rgb": [255, 0, 0],
            "target_kind": "application",
            "pid": 4242,
        },
        "unrelated": {
            "label": "unrelated-app-green",
            "placement": "other_application",
            "rgb": [0, 255, 0],
            "pid": 4243,
        },
    },
    "transport": {"kind": "webrtc"},
    "production_media_ready": True,
    "production_readiness": {
        "ready": True,
        "requires_production_codec": True,
        "production_codec_negotiated": True,
        "media_transport_ready": True,
        "client_media_ready": True,
    },
    "decoded_frames": {
        "count": 2,
        "rtp_packet_count": 10,
        "width": 3,
        "height": 3,
        "selected_content_present": True,
        "unrelated_sentinel_present": False,
        "full_display_leak_detected": False,
        "selected_pixel_count": 9,
        "unrelated_pixel_count": 0,
    },
    "artifacts": {
        "decoded_frame_sample": str(sample),
        "session_id": "rd-e2e-app-test",
        "binding_id": "tb_app_test",
        "binding_epoch": 1,
        "target_identity_epoch": 99,
        "target_geometry_revision": 1,
        "capture_scope": "AppSurface",
    },
}
with open(os.environ["EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON"], "w", encoding="utf-8") as f:
    json.dump(evidence, f)
PY
EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB="255,0,0" \
EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB="0,255,0" \
EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL="selected-app-red" \
EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL="unrelated-app-green" \
EASYNET_REMOTEAPP_SELECTED_SENTINEL_PID="4242" \
EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PID="4243" \
"$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  --run \
  --target-kind application \
  --probe-cmd "python3 '$PROBE_APP'" \
  --out-dir "$SANDBOX/e2e-app-out" >/dev/null

cat >"$SANDBOX/control.json" <<'JSON'
{
  "daemon_identity": {
    "mode": "device",
    "realm": "localhost",
    "node_id": "dev"
  },
  "daemon_version": "test",
  "pid": 1,
  "supported_ipc_versions": {
    "min": 1,
    "max": 1
  }
}
JSON

if EASYNET_HOST_REMOTEAPP_DECODED_FRAME_E2E=1 \
  EASYNET_REMOTEAPP_CONTROL_DISCOVERY_JSON="$SANDBOX/control.json" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
    --run \
    --out-dir "$SANDBOX/missing-receiver" >/dev/null 2>&1; then
  echo "remoteapp e2e harness accepted bundled probe without a frame receiver" >&2
  exit 1
fi

FAKE_EASYNET="$SANDBOX/fake-easynet"
cat >"$FAKE_EASYNET" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

case "$*" in
  "ability refresh-remote-targets --type window --format json")
    cat <<'JSON'
{
  "observed_at_ms": 123456789,
  "freshness_ttl_ms": 5000,
  "resources": [
    {
      "resource_ura": "easynet:///r/localhost/resource/device.dev/streams/window.test",
      "type": "window",
      "display_name": "EasyNet selected sentinel",
      "metadata": {
        "availability": "available",
        "app_name": "EasyNetProbe",
        "title": "duplicate-sentinel",
        "pid": 4242
      }
    },
    {
      "resource_ura": "easynet:///r/localhost/resource/device.dev/streams/window.other",
      "type": "window",
      "display_name": "EasyNet selected sentinel",
      "metadata": {
        "availability": "available",
        "app_name": "EasyNetProbe",
        "title": "duplicate-sentinel",
        "pid": 4243
      }
    }
  ]
}
JSON
    ;;
  "ability create-remote-desktop-session --subject easynet:///r/localhost/resource/device.dev/streams/window.test --mode view_only --transport webrtc --format json")
    cat <<'JSON'
{
  "session": {
    "session_id": "rd-e2e-probe",
    "target_binding": {
      "subject_ura": "easynet:///r/localhost/resource/device.dev/streams/window.test",
      "target_kind": "window",
      "capture_scope": "WindowSurface",
      "binding_id": "tb_test",
      "binding_epoch": 1,
      "target_identity_epoch": 1,
      "target_geometry_revision": 1,
      "media_source_epoch": 1,
      "consent_epoch": 1,
      "resolved_identity": {
        "window_id": 7,
        "pid": 4242
      }
    },
    "scope_audit": {
      "scope_widened": false,
      "display_fallback_used": false
    }
  },
  "invocation": {
    "ability": "remote_desktop.create_session",
    "subject_ura": "easynet:///r/localhost/resource/device.dev/streams/window.test",
    "callee_ura": "easynet:///r/localhost/agent/device.dev.remote-desktop",
    "request_id": "invocation-test"
  }
}
JSON
    ;;
  *)
    echo "unexpected fake easynet invocation: $*" >&2
    exit 1
    ;;
esac
SH
chmod +x "$FAKE_EASYNET"

FRAME_RECEIVER="$SANDBOX/fake_frame_receiver.py"
cat >"$FRAME_RECEIVER" <<'PY'
import json
import os
import pathlib

sample = pathlib.Path(os.environ["EASYNET_REMOTEAPP_FRAME_ANALYSIS_JSON"]).parent / "sample-frame.ppm"
sample.write_bytes(b"P6\n3 3\n255\n" + b"\xff\x00\x00" * 9)
with open(os.environ["EASYNET_REMOTEAPP_FRAME_ANALYSIS_JSON"], "w", encoding="utf-8") as f:
    json.dump(
        {
            "status": "passed",
            "transport": {"kind": "webrtc"},
            "production_media_ready": True,
            "production_readiness": {
                "ready": True,
                "requires_production_codec": True,
                "production_codec_negotiated": True,
                "media_transport_ready": True,
                "client_media_ready": True,
            },
            "decoded_frames": {
                "count": 2,
                "rtp_packet_count": 10,
                "width": 3,
                "height": 3,
                "selected_content_present": True,
                "unrelated_sentinel_present": False,
                "full_display_leak_detected": False,
                "selected_pixel_count": 9,
                "unrelated_pixel_count": 0,
            },
            "artifacts": {
                "decoded_frame_sample": str(sample),
                "session_id": "rd-e2e-probe",
                "binding_id": "tb_test",
                "binding_epoch": 1,
                "target_identity_epoch": 1,
                "target_geometry_revision": 1,
                "capture_scope": "WindowSurface",
            },
        },
        f,
    )
PY

EASYNET_REMOTEAPP_EASYNET_BIN="$FAKE_EASYNET" \
EASYNET_REMOTEAPP_FRAME_RECEIVER_CMD="python3 '$FRAME_RECEIVER'" \
EASYNET_REMOTEAPP_CONTROL_DISCOVERY_JSON="$SANDBOX/control.json" \
EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB="255,0,0" \
EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB="0,255,0" \
EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL="selected-window-red" \
EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL="unrelated-window-green" \
EASYNET_REMOTEAPP_TARGET_PID="4242" \
EASYNET_REMOTEAPP_SELECTED_SENTINEL_PID="4242" \
EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PID="4243" \
"$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  --run \
  --out-dir "$SANDBOX/bundled-probe-out" >/dev/null

cp "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good"
perl -0pi -e 's/client_media_ready/clientPresentingOmitted/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted missing client-presenting readiness assertion" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"

cp "$SANDBOX/examples/easynet-remoteapp-frame-receiver.rs" \
  "$SANDBOX/examples/easynet-remoteapp-frame-receiver.rs.good"
perl -0pi -e 's/report_client_presenting\(config, signal\.transport_epoch\)/report_client_presenting_omitted(config, signal.transport_epoch)/g' \
  "$SANDBOX/examples/easynet-remoteapp-frame-receiver.rs"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted receiver without client-presenting report" >&2
  exit 1
fi
mv "$SANDBOX/examples/easynet-remoteapp-frame-receiver.rs.good" \
  "$SANDBOX/examples/easynet-remoteapp-frame-receiver.rs"

cp "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good"
perl -0pi -e 's/unrelated_sentinel_present/unrelated_sentinel_omitted/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted missing sentinel exclusion assertion" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good"
perl -0pi -e 's/app_window_set/app_window_set_omitted/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted missing application window-set assertion" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good"
perl -0pi -e 's/window target must include target_binding\.resolved_identity/window target identity omitted/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted missing window resolved identity assertion" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good"
perl -0pi -e 's/artifacts\.target_identity_epoch/artifacts.target_identity_epoch_omitted/g; s/artifact target_identity_epoch/artifact identity epoch omitted/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted missing artifact target identity epoch assertion" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good"
perl -0pi -e 's/remote_desktop\.create_session/remote_desktop.open_session/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted missing create_session assertion" >&2
  exit 1
fi

mv "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good"
perl -0pi -e 's/sentinel_fixture/sentinel_fixture_omitted/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted missing dual-target sentinel fixture assertion" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh.good"
perl -0pi -e 's/EASYNET_REMOTEAPP_FRAME_RECEIVER_CMD/EASYNET_REMOTEAPP_FAKE_RECEIVER_CMD/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted bundled probe without receiver command contract" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh"

echo "test_check_remoteapp_e2e_acceptance_boundary.sh: all cases passed"
