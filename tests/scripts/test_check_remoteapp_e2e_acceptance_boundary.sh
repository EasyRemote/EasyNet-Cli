#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-e2e-acceptance-boundary.sh"
HARNESS="$REPO_ROOT/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
PROBE_HARNESS="$REPO_ROOT/tools/scripts/host-remoteapp-decoded-frame-probe.sh"
SENTINEL_FIXTURE="$REPO_ROOT/tools/scripts/host-remoteapp-sentinel-fixture.sh"
TARGET_FRESHNESS="$REPO_ROOT/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh"
CREATE_FAILCLOSED="$REPO_ROOT/tools/scripts/host-remoteapp-create-session-failclosed-e2e.sh"
PERMISSION_SUBJECT="$REPO_ROOT/tools/scripts/host-remoteapp-permission-subject-e2e.sh"
DISPLAY_FALLBACK_FORBIDDEN="$REPO_ROOT/tools/scripts/host-remoteapp-display-fallback-forbidden-e2e.sh"
WEAK_IDENTITY_AMBIGUITY="$REPO_ROOT/tools/scripts/host-remoteapp-weak-identity-ambiguity-e2e.sh"
VIEW_ONLY_INPUT_SAFETY="$REPO_ROOT/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh"
SESSION_TIMEOUT="$REPO_ROOT/tools/scripts/host-remoteapp-session-timeout-e2e.sh"
SESSION_CANCEL="$REPO_ROOT/tools/scripts/host-remoteapp-session-cancel-e2e.sh"
PERMISSION_REVOKE="$REPO_ROOT/tools/scripts/host-remoteapp-permission-revoke-e2e.sh"
SESSION_RESUME="$REPO_ROOT/tools/scripts/host-remoteapp-session-resume-e2e.sh"
LIFECYCLE_HARNESS_LIB="$REPO_ROOT/tools/scripts/remoteapp-lifecycle-harness-lib.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

mkdir -p "$SANDBOX/tools/scripts" "$SANDBOX/docs/design" "$SANDBOX/examples"
cp "$HARNESS" "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
cp "$PROBE_HARNESS" "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh"
cp "$SENTINEL_FIXTURE" "$SANDBOX/tools/scripts/host-remoteapp-sentinel-fixture.sh"
cp "$TARGET_FRESHNESS" "$SANDBOX/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh"
cp "$CREATE_FAILCLOSED" "$SANDBOX/tools/scripts/host-remoteapp-create-session-failclosed-e2e.sh"
cp "$PERMISSION_SUBJECT" "$SANDBOX/tools/scripts/host-remoteapp-permission-subject-e2e.sh"
cp "$DISPLAY_FALLBACK_FORBIDDEN" "$SANDBOX/tools/scripts/host-remoteapp-display-fallback-forbidden-e2e.sh"
cp "$WEAK_IDENTITY_AMBIGUITY" "$SANDBOX/tools/scripts/host-remoteapp-weak-identity-ambiguity-e2e.sh"
cp "$VIEW_ONLY_INPUT_SAFETY" "$SANDBOX/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh"
cp "$SESSION_TIMEOUT" "$SANDBOX/tools/scripts/host-remoteapp-session-timeout-e2e.sh"
cp "$SESSION_CANCEL" "$SANDBOX/tools/scripts/host-remoteapp-session-cancel-e2e.sh"
cp "$PERMISSION_REVOKE" "$SANDBOX/tools/scripts/host-remoteapp-permission-revoke-e2e.sh"
cp "$SESSION_RESUME" "$SANDBOX/tools/scripts/host-remoteapp-session-resume-e2e.sh"
cp "$LIFECYCLE_HARNESS_LIB" "$SANDBOX/tools/scripts/remoteapp-lifecycle-harness-lib.sh"
cp "$REPO_ROOT/examples/easynet-remoteapp-frame-receiver.rs" \
  "$SANDBOX/examples/easynet-remoteapp-frame-receiver.rs"
chmod +x "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
chmod +x "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh"
chmod +x "$SANDBOX/tools/scripts/host-remoteapp-sentinel-fixture.sh"
chmod +x "$SANDBOX/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh"
chmod +x "$SANDBOX/tools/scripts/host-remoteapp-create-session-failclosed-e2e.sh"
chmod +x "$SANDBOX/tools/scripts/host-remoteapp-permission-subject-e2e.sh"
chmod +x "$SANDBOX/tools/scripts/host-remoteapp-display-fallback-forbidden-e2e.sh"
chmod +x "$SANDBOX/tools/scripts/host-remoteapp-weak-identity-ambiguity-e2e.sh"
chmod +x "$SANDBOX/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh"
chmod +x "$SANDBOX/tools/scripts/host-remoteapp-session-timeout-e2e.sh"
chmod +x "$SANDBOX/tools/scripts/host-remoteapp-session-cancel-e2e.sh"
chmod +x "$SANDBOX/tools/scripts/host-remoteapp-permission-revoke-e2e.sh"
chmod +x "$SANDBOX/tools/scripts/host-remoteapp-session-resume-e2e.sh"
chmod +x "$SANDBOX/tools/scripts/remoteapp-lifecycle-harness-lib.sh"
cat >"$SANDBOX/docs/design/remoteapp-targeted-session-spec.md" <<'MD'
| E2E-01 target picker freshness | live refresh returns known target |
| E2E-02 permission subject correctness | invalid_argument for target Resource subjects |
| E2E-03 exact window session | decoded stream excludes sentinel |
| E2E-04 exact application session | decoded stream excludes other apps |
| E2E-05 stale window fail-closed | no active session row |
| E2E-06 no media re-resolution | media starts from stored target binding |
| E2E-07 display fallback forbidden | no decoded full display |
| E2E-08 move/resize tracking | ordered target geometry events |
| E2E-09 target loss vs transport failure | target loss is not transport failure |
| E2E-10 weak identity ambiguity | weak native identity fails closed |
| E2E-11 view-only input safety | input_mode=view_only and input_scope_unsupported |
MD

CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

"$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  --self-test \
  --target-kind window >/dev/null
"$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  --self-test \
  --target-kind application >/dev/null
"$SANDBOX/tools/scripts/host-remoteapp-create-session-failclosed-e2e.sh" \
  --self-test >/dev/null
"$SANDBOX/tools/scripts/host-remoteapp-permission-subject-e2e.sh" \
  --self-test >/dev/null
"$SANDBOX/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh" \
  --self-test >/dev/null
"$SANDBOX/tools/scripts/host-remoteapp-display-fallback-forbidden-e2e.sh" \
  --self-test >/dev/null
"$SANDBOX/tools/scripts/host-remoteapp-weak-identity-ambiguity-e2e.sh" \
  --self-test >/dev/null
"$SANDBOX/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh" \
  --self-test >/dev/null
"$SANDBOX/tools/scripts/host-remoteapp-session-timeout-e2e.sh" \
  --self-test >/dev/null
"$SANDBOX/tools/scripts/host-remoteapp-session-cancel-e2e.sh" \
  --self-test >/dev/null
"$SANDBOX/tools/scripts/host-remoteapp-permission-revoke-e2e.sh" \
  --self-test >/dev/null
"$SANDBOX/tools/scripts/host-remoteapp-session-resume-e2e.sh" \
  --self-test >/dev/null

cp "$SANDBOX/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh.good"
perl -0pi -e 's/run_easynet ability bidi "\$ATTACH_ABILITY_URA"/echo skipped remote desktop attach bidi probe/' \
  "$SANDBOX/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted view-only input harness without public attach Bidi probe" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh.good"
perl -0pi -e 's/"type": "input_applied"/"type": "input_application_unchecked"/' \
  "$SANDBOX/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh"
perl -0pi -e 's/view-only diagnostic input probe must not apply pointer or key frames/view-only diagnostic input probe accepts applied frames/' \
  "$SANDBOX/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted view-only input harness without applied-frame rejection guard" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-session-timeout-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-session-timeout-e2e.sh.good"
perl -0pi -e 's/show-remote-desktop-session/show-remote-desktop-status/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-session-timeout-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted session timeout harness without public show_session observation" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-session-timeout-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-session-timeout-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-session-timeout-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-session-timeout-e2e.sh.good"
perl -0pi -e 's/end_after_timeout must preserve the original timeout terminal receipt/end_after_timeout may replace timeout terminal receipt/' \
  "$SANDBOX/tools/scripts/host-remoteapp-session-timeout-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted session timeout harness without terminal receipt preservation" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-session-timeout-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-session-timeout-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-session-cancel-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-session-cancel-e2e.sh.good"
perl -0pi -e 's/remote_desktop\.end_session/remote_desktop.close_session/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-session-cancel-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted session cancel harness without public end_session invocation" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-session-cancel-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-session-cancel-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-session-cancel-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-session-cancel-e2e.sh.good"
perl -0pi -e 's/end_cancel_again must preserve the original cancel terminal receipt/end_cancel_again may replace cancel terminal receipt/' \
  "$SANDBOX/tools/scripts/host-remoteapp-session-cancel-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted session cancel harness without terminal receipt preservation" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-session-cancel-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-session-cancel-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-permission-revoke-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-permission-revoke-e2e.sh.good"
perl -0pi -e 's/real_platform_permission_revoke/synthetic_permission_revoke/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-permission-revoke-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted permission revoke harness without real platform proof mode" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-permission-revoke-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-permission-revoke-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-permission-revoke-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-permission-revoke-e2e.sh.good"
perl -0pi -e 's/TARGET_PERMISSION_REVOKED/TARGET_PERMISSION_SUSPENDED/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-permission-revoke-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted permission revoke harness without revoked target event evidence" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-permission-revoke-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-permission-revoke-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-session-resume-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-session-resume-e2e.sh.good"
perl -0pi -e 's/remote_desktop\.refresh_lease/remote_desktop.recreate_session/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-session-resume-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted session resume harness without public refresh_lease" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-session-resume-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-session-resume-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-session-resume-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-session-resume-e2e.sh.good"
perl -0pi -e 's/show_after_original_lease must prove the refreshed session survived/show_after_original_lease may create a replacement session/' \
  "$SANDBOX/tools/scripts/host-remoteapp-session-resume-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted session resume harness without same-session survival proof" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-session-resume-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-session-resume-e2e.sh"

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

cp "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good"
perl -0pi -e 's/permission_preflight_output="\$\(preflight_bundled_probe_permissions 2>&1\)"/permission_preflight_output="$(true 2>\&1)"/' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted decoded-frame harness without permission preflight call" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh.good"
perl -0pi -e 's/preflight_output="\$\(preflight_daemon_invocation_ready 2>&1\)"/preflight_output="$(true 2>\&1)"/' \
  "$SANDBOX/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted target picker harness without daemon invocation preflight call" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh"

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
        "args": {
            "mode": "view_only",
            "transport": "webrtc"
        },
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
        "subject_ura": subject,
        "session_id": "rd-e2e-test",
        "binding_id": "tb_test",
        "binding_epoch": 1,
        "target_identity_epoch": 1,
        "target_geometry_revision": 1,
        "media_source_epoch": 1,
        "consent_epoch": 1,
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

if EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB="255,0,0" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB="0,255,0" \
  EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL="selected-window-red" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL="unrelated-window-green" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
    --run \
    --probe-cmd "python3 '$PROBE'" \
    --lifecycle-scenario move-resize \
    --out-dir "$SANDBOX/e2e-missing-lifecycle" >/dev/null 2>&1; then
  echo "remoteapp e2e harness accepted lifecycle scenario evidence without lifecycle events" >&2
  exit 1
fi

PROBE_ARGS_SUBJECT="$SANDBOX/fake_probe_args_subject.py"
cp "$PROBE" "$PROBE_ARGS_SUBJECT"
perl -0pi -e 's/"transport": "webrtc"/"transport": "webrtc",\n            "subject_ura": subject/' \
  "$PROBE_ARGS_SUBJECT"
if EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB="255,0,0" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB="0,255,0" \
  EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL="selected-window-red" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL="unrelated-window-green" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
    --run \
    --probe-cmd "python3 '$PROBE_ARGS_SUBJECT'" \
    --out-dir "$SANDBOX/e2e-args-subject" >/dev/null 2>&1; then
  echo "remoteapp e2e harness accepted create_session subject identity inside invocation args" >&2
  exit 1
fi

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
perl -0pi -e 's/("artifacts": \{[^}]*?        "binding_epoch": 1,)\n        "target_identity_epoch": 1,\n        "target_geometry_revision": 1,/$1/s' \
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

PROBE_NO_ARTIFACT_MEDIA_EPOCH="$SANDBOX/fake_probe_no_artifact_media_epoch.py"
cp "$PROBE" "$PROBE_NO_ARTIFACT_MEDIA_EPOCH"
perl -0pi -e 's/("artifacts": \{[^}]*?)\n        "media_source_epoch": 1,/$1/s' \
  "$PROBE_NO_ARTIFACT_MEDIA_EPOCH"
if EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB="255,0,0" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB="0,255,0" \
  EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL="selected-window-red" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL="unrelated-window-green" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
    --run \
    --probe-cmd "python3 '$PROBE_NO_ARTIFACT_MEDIA_EPOCH'" \
    --out-dir "$SANDBOX/e2e-no-artifact-media-epoch" >/dev/null 2>&1; then
  echo "remoteapp e2e harness accepted decoded artifact without media source epoch binding" >&2
  exit 1
fi

PROBE_NO_ARTIFACT_SUBJECT="$SANDBOX/fake_probe_no_artifact_subject.py"
cp "$PROBE" "$PROBE_NO_ARTIFACT_SUBJECT"
perl -0pi -e 's/("artifacts": \{[^}]*?)\n        "subject_ura": subject,/$1/s' \
  "$PROBE_NO_ARTIFACT_SUBJECT"
if EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB="255,0,0" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB="0,255,0" \
  EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL="selected-window-red" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL="unrelated-window-green" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
    --run \
    --probe-cmd "python3 '$PROBE_NO_ARTIFACT_SUBJECT'" \
    --out-dir "$SANDBOX/e2e-no-artifact-subject" >/dev/null 2>&1; then
  echo "remoteapp e2e harness accepted decoded artifact without target subject binding" >&2
  exit 1
fi

PROBE_NO_ARTIFACT_CONSENT_EPOCH="$SANDBOX/fake_probe_no_artifact_consent_epoch.py"
cp "$PROBE" "$PROBE_NO_ARTIFACT_CONSENT_EPOCH"
perl -0pi -e 's/("artifacts": \{[^}]*?)\n        "consent_epoch": 1,/$1/s' \
  "$PROBE_NO_ARTIFACT_CONSENT_EPOCH"
if EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB="255,0,0" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB="0,255,0" \
  EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL="selected-window-red" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL="unrelated-window-green" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
    --run \
    --probe-cmd "python3 '$PROBE_NO_ARTIFACT_CONSENT_EPOCH'" \
    --out-dir "$SANDBOX/e2e-no-artifact-consent-epoch" >/dev/null 2>&1; then
  echo "remoteapp e2e harness accepted decoded artifact without consent epoch binding" >&2
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
        "args": {
            "mode": "view_only",
            "transport": "webrtc",
        },
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
        "subject_ura": subject,
        "session_id": "rd-e2e-no-client-ready",
        "binding_id": "tb_no_client_ready",
        "binding_epoch": 1,
        "target_identity_epoch": 1,
        "target_geometry_revision": 1,
        "media_source_epoch": 1,
        "consent_epoch": 1,
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
        "args": {
            "mode": "view_only",
            "transport": "webrtc",
        },
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
        "subject_ura": subject,
        "session_id": "rd-e2e-fixture-test",
        "binding_id": "tb_fixture_test",
        "binding_epoch": 1,
        "target_identity_epoch": 1,
        "target_geometry_revision": 1,
        "media_source_epoch": 1,
        "consent_epoch": 1,
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
python3 - "$SANDBOX/e2e-preflight-out/report.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as f:
    report = json.load(f)
if report.get("status") != "failed":
    raise SystemExit("remoteapp e2e preflight failure did not write failed status")
if "bundled_probe_preflight_failed" not in str(report.get("reason", "")):
    raise SystemExit("remoteapp e2e preflight failure did not preserve preflight failure reason")
PY

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
        "args": {
            "mode": "view_only",
            "transport": "webrtc",
        },
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
            "display_id": 1,
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
        "subject_ura": subject,
        "session_id": "rd-e2e-app-test",
        "binding_id": "tb_app_test",
        "binding_epoch": 1,
        "target_identity_epoch": 99,
        "target_geometry_revision": 1,
        "media_source_epoch": 1,
        "consent_epoch": 1,
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

PROBE_APP_DISPLAY_MISMATCH="$SANDBOX/fake_probe_application_display_mismatch.py"
cp "$PROBE_APP" "$PROBE_APP_DISPLAY_MISMATCH"
perl -0pi -e 's/"display_id": 1,\n            "pid": 4242/"display_id": 99,\n            "pid": 4242/' \
  "$PROBE_APP_DISPLAY_MISMATCH"
if EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB="255,0,0" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB="0,255,0" \
  EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL="selected-app-red" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL="unrelated-app-green" \
  EASYNET_REMOTEAPP_SELECTED_SENTINEL_PID="4242" \
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PID="4243" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
    --run \
    --target-kind application \
    --probe-cmd "python3 '$PROBE_APP_DISPLAY_MISMATCH'" \
    --out-dir "$SANDBOX/e2e-app-display-mismatch" >/dev/null 2>&1; then
  echo "remoteapp e2e harness accepted application resolved identity display mismatch" >&2
  exit 1
fi

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
  EASYNET_REMOTEAPP_EASYNET_COMMAND_TIMEOUT_SEC=1 \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
    --run \
    --out-dir "$SANDBOX/missing-receiver" >/dev/null 2>&1; then
  echo "remoteapp e2e harness accepted bundled probe without a frame receiver" >&2
  exit 1
fi

FAKE_EASYNET_HANGS="$SANDBOX/fake-easynet-hangs"
cat >"$FAKE_EASYNET_HANGS" <<'SH'
#!/usr/bin/env bash
python3 -c 'import time; time.sleep(10)'
SH
chmod +x "$FAKE_EASYNET_HANGS"
if EASYNET_REMOTEAPP_EASYNET_BIN="$FAKE_EASYNET_HANGS" \
  EASYNET_REMOTEAPP_EASYNET_COMMAND_TIMEOUT_SEC=1 \
  "$SANDBOX/tools/scripts/host-remoteapp-permission-subject-e2e.sh" \
    --run \
    --out-dir "$SANDBOX/permission-timeout" >/dev/null 2>&1; then
  echo "remoteapp permission preflight accepted a hanging easynet command" >&2
  exit 1
fi

FAKE_EASYNET="$SANDBOX/fake-easynet"
cat >"$FAKE_EASYNET" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

case "$*" in
  "runtime status --json")
    cat <<'JSON'
{
  "daemon": {
    "control_accepting": true,
    "invocation_accepting": true,
    "pid_alive": true
  },
  "connection": {
    "device_ura": "easynet:///r/localhost/device/dev",
    "node_id": "dev",
    "state": "online"
  }
}
JSON
    ;;
  "ability list --format json --pattern remote_desktop.*")
    cat <<'JSON'
[
  {
    "name": "remote_desktop.permission_status",
    "descriptor_ref": "easynet:///r/localhost/ability/system-agent.dev.remote-desktop.remote_desktop.permission_status@1.0.0#test!read",
    "owner_ura": "easynet:///r/localhost/agent/device.dev.remote-desktop",
    "metadata": {
      "subject_contract_ura": "easynet:///r/_system/resource/ability-contract.remote-desktop/host-local-permission-subject"
    },
    "scope_subjects": {
      "uras": ["agent", "resource", "user"]
    }
  },
  {
    "name": "remote_desktop.request_permission",
    "descriptor_ref": "easynet:///r/localhost/ability/system-agent.dev.remote-desktop.remote_desktop.request_permission@1.0.0#test!write",
    "owner_ura": "easynet:///r/localhost/agent/device.dev.remote-desktop",
    "metadata": {
      "subject_contract_ura": "easynet:///r/_system/resource/ability-contract.remote-desktop/host-local-permission-subject"
    },
    "scope_subjects": {
      "uras": ["agent", "resource", "user"]
    }
  }
]
JSON
    ;;
  ability\ invoke\ *remote_desktop.permission_status*\ --node\ *\ --subject\ easynet:///r/localhost/resource/user.*/invoke/remote_desktop.permission_status\ *)
    cat <<'JSON'
{
  "granted": true,
  "permission": "screen_capture",
  "process_path": "/tmp/easynet-daemon-test",
  "settings_hint": "System Settings > Privacy & Security > Screen & System Audio Recording",
  "subject_contract": {
    "subject_contract_ura": "easynet:///r/_system/resource/ability-contract.remote-desktop/host-local-permission-subject",
    "allowed_subjects": [
      "caller_user_self",
      "descriptor_bound_invoke_resource",
      "local_system_loopback"
    ],
    "target_resource_subjects_allowed": false
  }
}
JSON
    ;;
  ability\ invoke\ *remote_desktop.request_permission*\ --node\ *\ --subject\ easynet:///r/localhost/resource/user.*/invoke/remote_desktop.request_permission\ *)
    cat <<'JSON'
{
  "granted": true,
  "permission": "screen_capture",
  "process_path": "/tmp/easynet-daemon-test",
  "settings_hint": "System Settings > Privacy & Security > Screen & System Audio Recording",
  "subject_contract": {
    "subject_contract_ura": "easynet:///r/_system/resource/ability-contract.remote-desktop/host-local-permission-subject",
    "allowed_subjects": [
      "caller_user_self",
      "descriptor_bound_invoke_resource",
      "local_system_loopback"
    ],
    "target_resource_subjects_allowed": false
  }
}
JSON
    ;;
  ability\ invoke\ *remote_desktop.permission_status*\ --node\ *\ --subject\ easynet:///r/localhost/resource/device.dev/streams/*\ *)
    echo "remote_desktop.permission_status: screen-capture permission probes are host-local and MUST NOT be scoped to a remote desktop resource subject; reason=invalid_argument" >&2
    exit 1
    ;;
  ability\ invoke\ *remote_desktop.request_permission*\ --node\ *\ --subject\ easynet:///r/localhost/resource/device.dev/streams/*\ *)
    echo "remote_desktop.request_permission: screen-capture permission probes are host-local and MUST NOT be scoped to a remote desktop resource subject; reason=invalid_argument" >&2
    exit 1
    ;;
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
    "args": {
      "mode": "view_only",
      "transport_preferences": ["webrtc"],
      "consent_ticket": "ticket-from-grant"
    },
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
                "subject_ura": "easynet:///r/localhost/resource/device.dev/streams/window.test",
                "session_id": "rd-e2e-probe",
                "binding_id": "tb_test",
                "binding_epoch": 1,
                "target_identity_epoch": 1,
                "target_geometry_revision": 1,
                "media_source_epoch": 1,
                "consent_epoch": 1,
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

cp "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh.good"
perl -0pi -e 's/prepare_bundled_frame_receiver/prepare_frame_receiver_after_session/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted probe without pre-session frame receiver preparation" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh"

FAKE_EASYNET_NO_INVOCATION_ARGS="$SANDBOX/fake-easynet-no-invocation-args"
cp "$FAKE_EASYNET" "$FAKE_EASYNET_NO_INVOCATION_ARGS"
perl -0pi -e 's/\n    "args": \{\n      "mode": "view_only",\n      "transport_preferences": \["webrtc"\],\n      "consent_ticket": "ticket-from-grant"\n    \},//' \
  "$FAKE_EASYNET_NO_INVOCATION_ARGS"
if EASYNET_REMOTEAPP_EASYNET_BIN="$FAKE_EASYNET_NO_INVOCATION_ARGS" \
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
    --out-dir "$SANDBOX/bundled-probe-missing-args" >/dev/null 2>&1; then
  echo "remoteapp host E2E accepted bundled probe evidence without verified invocation args" >&2
  exit 1
fi

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
perl -0pi -e 's/artifacts\.media_source_epoch/artifacts.media_source_epoch_omitted/g; s/artifact media_source_epoch/artifact media source epoch omitted/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted missing artifact media source epoch assertion" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good"
perl -0pi -e 's/artifacts\.subject_ura/artifacts.subject_ura_omitted/g; s/artifact subject_ura/artifact subject omitted/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted missing artifact subject assertion" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"

cp "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh.good"
perl -0pi -e 's/artifacts\.consent_epoch/artifacts.consent_epoch_omitted/g; s/artifact consent_epoch/artifact consent epoch omitted/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted missing artifact consent epoch assertion" >&2
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
perl -0pi -e 's/contains_create_session_subject_arg/contains_create_session_subject_arg_omitted/g' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted missing create_session args subject rejection" >&2
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

cp "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh.good"
perl -0pi -e 's/"args": invocation\.get\("args"\),//' \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh"
if CHECK_REMOTEAPP_E2E_ACCEPTANCE_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp e2e acceptance checker accepted bundled probe without invocation args preservation" >&2
  exit 1
fi
mv "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh.good" \
  "$SANDBOX/tools/scripts/host-remoteapp-decoded-frame-probe.sh"

echo "test_check_remoteapp_e2e_acceptance_boundary.sh: all cases passed"
