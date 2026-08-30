#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"

fail() {
  printf 'test_frontend_remoteapp_browser_lifecycle_e2e: %s\n' "$1" >&2
  exit 1
}

"$SCRIPT" --self-test >/dev/null

OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

"$SCRIPT" --out-dir "$OUT_DIR/skip" >/tmp/frontend-remoteapp-browser-lifecycle-skip.out
grep -q '"status": "skipped"' "$OUT_DIR/skip/report.json" || \
  fail "default mode must emit skipped report"
grep -q "skipped" /tmp/frontend-remoteapp-browser-lifecycle-skip.out || \
  fail "default output must be explicit skipped evidence"

EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_OUT_DIR="$OUT_DIR/good" "$SCRIPT" --self-test >/dev/null
grep -q '"evidence_origin": "contract_self_test"' "$OUT_DIR/good/report.json" || \
  fail "self-test report must remain contract-only evidence"

if "$SCRIPT" --run --evidence-json "$OUT_DIR/good/evidence.json" \
    --out-dir "$OUT_DIR/self-test-as-live" >/tmp/frontend-remoteapp-browser-lifecycle-origin.out 2>&1; then
  fail "verifier accepted contract self-test evidence in run mode"
fi
grep -q "evidence_origin must be live_runner" /tmp/frontend-remoteapp-browser-lifecycle-origin.out || \
  fail "self-test provenance rejection was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/permission-request-good.json" "$OUT_DIR/permission-request-target-scoped.json" <<'PY'
import copy
import json
import sys

source, valid_path, invalid_path = sys.argv[1:]
evidence = json.load(open(source, encoding="utf-8"))
evidence["evidence_origin"] = "live_runner"
permission_request = {
    "name": "permission_requested",
    "status": "passed",
    "evidence_source": "browser_automation",
    "component_snapshot_only": False,
    "observed_at_ms": 1787332000045,
    "ability": "remote_desktop.request_permission",
    "subject_ura": None,
    "capture_state": "granted",
    "input_state": "blocked",
}
evidence["steps"].insert(4, permission_request)
json.dump(evidence, open(valid_path, "w", encoding="utf-8"), indent=2)

invalid = copy.deepcopy(evidence)
invalid["steps"][4]["subject_ura"] = invalid["selected_resource_ura"]
json.dump(invalid, open(invalid_path, "w", encoding="utf-8"), indent=2)
PY

"$SCRIPT" --run --evidence-json "$OUT_DIR/permission-request-good.json" \
  --out-dir "$OUT_DIR/permission-request-good" >/dev/null
if "$SCRIPT" --run --evidence-json "$OUT_DIR/permission-request-target-scoped.json" \
    --out-dir "$OUT_DIR/permission-request-target-scoped" \
    >/tmp/frontend-remoteapp-browser-lifecycle-permission-request-subject.out 2>&1; then
  fail "verifier accepted target-scoped request_permission evidence"
fi
grep -q "permission_requested must be host-local and not target-scoped" \
  /tmp/frontend-remoteapp-browser-lifecycle-permission-request-subject.out || \
  fail "request_permission subject rejection was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/target-scope-view-only.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["evidence_origin"] = "live_runner"
evidence["selected_target_kind"] = "window"
for step in evidence["steps"]:
    if step["name"] == "input_control_attempted_or_policy_blocked":
        step["blocked_reason"] = "target_scoped_keyboard_pointer_dispatch_unsafe"
        step["target_tracking"]["input_enabled"] = True
        step["target_tracking"]["input_blocked_reason"] = ""
    elif step["name"] == "input_control_after_resume":
        step["blocked_reason"] = "target_scoped_keyboard_pointer_dispatch_unsafe"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
"$SCRIPT" --run --evidence-json "$OUT_DIR/target-scope-view-only.json" \
  --out-dir "$OUT_DIR/target-scope-view-only" >/dev/null
grep -q '"status": "passed"' "$OUT_DIR/target-scope-view-only/report.json" || \
  fail "target-scope view-only policy must remain distinct from target lifecycle readiness"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/input-applied.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["evidence_origin"] = "live_runner"
session_id = evidence["session_id"]
for step in evidence["steps"]:
    if step["name"] != "input_control_attempted_or_policy_blocked":
        continue
    step.clear()
    step.update({
        "name": "input_control_attempted_or_policy_blocked",
        "status": "passed",
        "evidence_source": "browser_automation",
        "component_snapshot_only": False,
        "observed_at_ms": 1787332000110,
        "result": "input_applied",
        "visible_status": "input interactive ready",
        "client_sequence": 7,
        "target_focus_epoch": 11,
        "target_geometry_revision": 23,
        "latency_ms": 17,
        "submitted_frame": {
            "type": "pointer",
            "action": "down",
            "client_sequence": 7,
            "sent_at_ms": 1787332000100,
            "target_focus_epoch": 11,
            "target_geometry_revision": 23,
        },
        "interaction_sequence": [
            {
                "client_sequence": sequence,
                "target_focus_epoch": 11,
                "target_geometry_revision": 23 if frame_type == "pointer" else None,
                "latency_ms": 17,
                "submitted_frame": {
                    "type": frame_type,
                    "action": action,
                    "client_sequence": sequence,
                    "sent_at_ms": 1787332000100 + sequence,
                    "target_focus_epoch": 11,
                    **({"target_geometry_revision": 23} if frame_type == "pointer" else {}),
                },
                "applied_event": {
                    "event_type": "INPUT_FRAME_APPLIED",
                    "session_id": session_id,
                    "client_sequence": sequence,
                    "target_focus_epoch": 11,
                    **({"target_geometry_revision": 23} if frame_type == "pointer" else {}),
                },
            }
            for sequence, frame_type, action in [
                (7, "pointer", "down"),
                (8, "pointer", "up"),
                (9, "key", "down"),
                (10, "key", "up"),
            ]
        ],
        "applied_event": {
            "event_type": "INPUT_FRAME_APPLIED",
            "session_id": session_id,
            "client_sequence": 7,
            "target_focus_epoch": 11,
            "target_geometry_revision": 23,
        },
    })
for step in evidence["steps"]:
    if step["name"] != "input_control_after_resume":
        continue
    step.clear()
    step.update({
        "name": "input_control_after_resume",
        "status": "passed",
        "evidence_source": "browser_automation",
        "component_snapshot_only": False,
        "observed_at_ms": 1787332000170,
        "result": "input_applied",
        "visible_status": "input interactive ready",
        "client_sequence": 11,
        "target_focus_epoch": 12,
        "target_geometry_revision": 24,
        "latency_ms": 18,
        "submitted_frame": {
            "type": "key",
            "action": "down",
            "code": "KeyB",
            "client_sequence": 11,
            "sent_at_ms": 1787332000160,
            "target_focus_epoch": 12,
            "target_geometry_revision": 24,
        },
        "applied_event": {
            "event_type": "INPUT_FRAME_APPLIED",
            "session_id": session_id,
            "client_sequence": 11,
            "target_focus_epoch": 12,
            "target_geometry_revision": 24,
        },
    })
evidence["transport_resume"]["input_result_before"] = "input_applied"
evidence["transport_resume"]["input_result_after"] = "input_applied"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
"$SCRIPT" --run --evidence-json "$OUT_DIR/input-applied.json" --out-dir "$OUT_DIR/input-applied" >/dev/null
grep -q '"status": "passed"' "$OUT_DIR/input-applied/report.json" || \
  fail "input_applied evidence with focus epoch must pass"

python3 - "$OUT_DIR/input-applied.json" "$OUT_DIR/stale-resume-input.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
for step in evidence["steps"]:
    if step["name"] == "input_control_after_resume":
        step["client_sequence"] = 7
        step["submitted_frame"]["client_sequence"] = 7
        step["applied_event"]["client_sequence"] = 7
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/stale-resume-input.json" --out-dir "$OUT_DIR/stale-resume-input" >/tmp/frontend-remoteapp-browser-lifecycle-stale-resume-input.out 2>&1; then
  fail "verifier accepted post-resume input without sequence advancement"
fi
grep -q "post-resume input client_sequence must advance" /tmp/frontend-remoteapp-browser-lifecycle-stale-resume-input.out || \
  fail "stale post-resume input failure was not explicit"

python3 - "$OUT_DIR/input-applied.json" "$OUT_DIR/missing-submitted-focus.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
for step in evidence["steps"]:
    if step["name"] == "input_control_attempted_or_policy_blocked":
        del step["submitted_frame"]["target_focus_epoch"]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/missing-submitted-focus.json" --out-dir "$OUT_DIR/missing-submitted-focus" >/tmp/frontend-remoteapp-browser-lifecycle-missing-submitted-focus.out 2>&1; then
  fail "verifier accepted input_applied evidence without submitted_frame target_focus_epoch"
fi
grep -q "submitted_frame target_focus_epoch must match input_applied target_focus_epoch" /tmp/frontend-remoteapp-browser-lifecycle-missing-submitted-focus.out || \
  fail "missing submitted target_focus_epoch failure was not explicit"

python3 - "$OUT_DIR/input-applied.json" "$OUT_DIR/wrong-applied-focus.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
for step in evidence["steps"]:
    if step["name"] == "input_control_attempted_or_policy_blocked":
        step["applied_event"]["target_focus_epoch"] = 12
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/wrong-applied-focus.json" --out-dir "$OUT_DIR/wrong-applied-focus" >/tmp/frontend-remoteapp-browser-lifecycle-wrong-applied-focus.out 2>&1; then
  fail "verifier accepted input_applied evidence with stale applied_event target_focus_epoch"
fi
grep -q "applied_event target_focus_epoch must match input_applied target_focus_epoch" /tmp/frontend-remoteapp-browser-lifecycle-wrong-applied-focus.out || \
  fail "stale applied target_focus_epoch failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/blurred-policy-block.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["evidence_origin"] = "live_runner"
evidence["selected_target_kind"] = "window"
for step in evidence["steps"]:
    if step["name"] == "input_control_attempted_or_policy_blocked":
        step["blocked_reason"] = "target_blurred"
        step["target_tracking"]["input_enabled"] = False
        step["target_tracking"]["input_blocked_reason"] = "target_blurred"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/blurred-policy-block.json" --out-dir "$OUT_DIR/blurred-policy-block" >/tmp/frontend-remoteapp-browser-lifecycle-blurred-policy.out 2>&1; then
  fail "verifier accepted recoverable target_blurred state as completed input evidence"
fi
grep -q "policy_blocked input must expose a known blocked_reason" /tmp/frontend-remoteapp-browser-lifecycle-blurred-policy.out || \
  fail "recoverable target_blurred rejection was not explicit"

python3 - "$OUT_DIR/input-applied.json" "$OUT_DIR/focus-recovery.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
for step in evidence["steps"]:
    if step["name"] == "input_control_attempted_or_policy_blocked":
        step["focus_recovery"] = {
            "ability": "remote_desktop.focus_target",
            "invocation_observed": True,
            "invocation_count": 1,
            "prior_target_focus_epoch": 10,
            "committed_target_focus_epoch": 11,
        }
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
"$SCRIPT" --run --evidence-json "$OUT_DIR/focus-recovery.json" --out-dir "$OUT_DIR/focus-recovery" >/dev/null
grep -q '"status": "passed"' "$OUT_DIR/focus-recovery/report.json" || \
  fail "focus recovery evidence with an advancing committed epoch must pass"

python3 - "$OUT_DIR/focus-recovery.json" "$OUT_DIR/stale-focus-recovery.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
for step in evidence["steps"]:
    if step["name"] == "input_control_attempted_or_policy_blocked":
        step["focus_recovery"]["committed_target_focus_epoch"] = 10
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/stale-focus-recovery.json" --out-dir "$OUT_DIR/stale-focus-recovery" >/tmp/frontend-remoteapp-browser-lifecycle-stale-focus-recovery.out 2>&1; then
  fail "verifier accepted focus recovery without a committed epoch advance"
fi
grep -q "committed_target_focus_epoch must match input_applied target_focus_epoch" /tmp/frontend-remoteapp-browser-lifecycle-stale-focus-recovery.out || \
  fail "stale focus recovery failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/application-target.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["evidence_origin"] = "live_runner"
evidence["selected_target_kind"] = "application"
evidence["selected_target_snapshot"] = {
    "resource_ura": evidence["selected_resource_ura"],
    "type": "application",
    "metadata": {
        "capture_target": "application",
        "discovery_scope": "application_window_set",
        "display_scoped": False,
        "display_id": None,
        "resolved_window_ids": [101, 202],
        "window_count": 2,
        "window_set_epoch": 17,
        "surface_layout_epoch": 19,
        "platform": "linux",
    },
}
evidence["target_execution_snapshot"] = {
    "observed_at_ms": 1787332000105,
    "source_ability": "remote_desktop.show_session",
    "session_id": evidence["session_id"],
    "subject_ura": evidence["selected_resource_ura"],
    "target_binding": {
        "subject_ura": evidence["selected_resource_ura"],
        "target_kind": "application",
        "binding_epoch": 1,
        "capture_scope": "AppSurface",
        "native_locator": {"display_id": None},
        "backend": "xcap",
        "capture_proof": {
            "target_kind": "application",
            "backend": "xcap",
            "display_id": None,
            "native_width": 800,
            "native_height": 300,
            "verified_at_ms": 100,
            "app_window_set": {"resolved_window_ids": [101, 202]},
            "app_surface_layout": {
                "front_to_back_surfaces": [
                    {"window_id": 101, "x": 0, "y": 0, "width": 400, "height": 300},
                    {"window_id": 202, "x": 400, "y": 0, "width": 400, "height": 300},
                ],
            },
        },
        "app_window_set": {
            "resolved_window_ids": [101, 202],
            "window_set_epoch": 17,
        },
        "app_surface_layout": {
            "front_to_back_surfaces": [
                {"window_id": 101, "x": 0, "y": 0, "width": 400, "height": 300},
                {"window_id": 202, "x": 400, "y": 0, "width": 400, "height": 300},
            ],
        },
    },
    "scope_audit": {
        "requested_target_kind": "application",
        "effective_target_kind": "application",
        "capture_surface": "AppSurface",
        "scope_widened": False,
        "display_fallback_used": False,
    },
}
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
"$SCRIPT" --run --evidence-json "$OUT_DIR/application-target.json" --out-dir "$OUT_DIR/application-target" >/dev/null
grep -q '"status": "passed"' "$OUT_DIR/application-target/report.json" || \
  fail "exact application window-set snapshot evidence must pass"

python3 - "$OUT_DIR/application-target.json" "$OUT_DIR/application-target-churn.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
session_id = evidence["session_id"]
subject = evidence["selected_resource_ura"]
for key in ("transport_resume", "transport_snapshots", "transport_disconnect_observations"):
    evidence.pop(key, None)
resume_steps = {
    "transport_disconnected",
    "session_preserved_for_reconnect",
    "transport_reconnected",
    "watch_events_reestablished",
    "media_presented_after_resume",
    "input_control_after_resume",
}
evidence["steps"] = [step for step in evidence["steps"] if step["name"] not in resume_steps]
input_step = next(step for step in evidence["steps"]
                  if step["name"] == "input_control_attempted_or_policy_blocked")
input_at = input_step["observed_at_ms"]
ended_index = next(i for i, step in enumerate(evidence["steps"])
                   if step["name"] == "session_ended")
interactions = []
for sequence, frame_type, action in [
    (21, "pointer", "down"),
    (22, "pointer", "up"),
    (23, "key", "down"),
    (24, "key", "up"),
]:
    interactions.append({
        "client_sequence": sequence,
        "target_focus_epoch": 7,
        "target_geometry_revision": 2,
        "latency_ms": 8,
        "submitted_frame": {
            "type": frame_type,
            "action": action,
            "client_sequence": sequence,
            "sent_at_ms": input_at + sequence,
            "target_focus_epoch": 7,
            **({"target_geometry_revision": 2} if frame_type == "pointer" else {}),
        },
        "applied_event": {
            "event_type": "INPUT_FRAME_APPLIED",
            "session_id": session_id,
            "client_sequence": sequence,
            "target_focus_epoch": 7,
            "target_geometry_revision": 2,
        },
    })
churn_input = {
    "result": "input_applied",
    "visible_status": "input scope target_local · pointer+keyboard",
    "client_sequence": 21,
    "target_focus_epoch": 7,
    "target_geometry_revision": 2,
    "latency_ms": 8,
    "submitted_frame": interactions[0]["submitted_frame"],
    "applied_event": interactions[0]["applied_event"],
    "interaction_sequence": interactions,
    "input_probe": {
        "source": "committed_application_surface_center",
        "window_id": 101,
    },
}
evidence["steps"][ended_index:ended_index] = [
    {
        "name": "media_presented_after_application_window_set_rebind",
        "status": "passed",
        "evidence_source": "browser_automation",
        "component_snapshot_only": False,
        "observed_at_ms": input_at + 10,
        "subject_ura": subject,
        "session_id": session_id,
        "binding_epoch": 2,
        "target_identity_epoch": 22,
        "frame_presented": True,
        "media_element_visible": True,
        "frames_presented": 3,
    },
    {
        "name": "input_applied_after_application_window_set_rebind",
        "status": "passed",
        "evidence_source": "browser_automation",
        "component_snapshot_only": False,
        "observed_at_ms": input_at + 20,
        **churn_input,
    },
    {
        "name": "application_window_set_rebound",
        "status": "passed",
        "evidence_source": "browser_automation",
        "component_snapshot_only": False,
        "observed_at_ms": input_at + 30,
        "subject_ura": subject,
        "session_id": session_id,
        "binding_epoch_before": 1,
        "binding_epoch_after": 2,
        "target_identity_epoch_before": 17,
        "target_identity_epoch_after": 22,
        "target_geometry_revision_before": 1,
        "target_geometry_revision_after": 2,
        "resolved_window_ids": [101, 303],
        "frames_rendered_after_rebind": 2,
        "input_applied_after_rebind": True,
        "scope_widened": False,
        "display_fallback_used": False,
    },
]
evidence["application_target_churn"] = {
    "proof_mode": "real_application_window_set_churn",
    "churn_mode": "window_set",
    "selected_resource_ura": subject,
    "session_id": session_id,
    "binding_epoch_before": 1,
    "binding_epoch_after": 2,
    "target_identity_epoch_before": 17,
    "target_identity_epoch_after": 22,
    "target_geometry_revision_before": 1,
    "target_geometry_revision_after": 2,
    "resolved_window_ids_before": [101, 202],
    "resolved_window_ids_after": [101, 303],
    "target_events": [],
    "frames_rendered_after_rebind": 2,
    "input_after_rebind": churn_input,
    "scope_widened": False,
    "display_fallback_used": False,
}
evidence["target_execution_snapshots"] = [
    evidence["target_execution_snapshot"],
    {
      "source_ability": "remote_desktop.show_session",
      "session_id": session_id,
      "subject_ura": subject,
      "target_binding": {
        "binding_epoch": 2,
        "app_window_set": {"resolved_window_ids": [101, 303]},
        "app_surface_layout": {
            "front_to_back_surfaces": [
                {"window_id": 101, "x": 10, "y": 20, "width": 400, "height": 300},
                {"window_id": 303, "x": 410, "y": 20, "width": 400, "height": 300},
            ],
        },
        "capture_proof": {
            "target_kind": "application",
            "backend": "xcap",
            "display_id": None,
            "native_width": 800,
            "native_height": 300,
            "verified_at_ms": 200,
            "app_window_set": {"resolved_window_ids": [101, 303]},
            "app_surface_layout": {
                "front_to_back_surfaces": [
                    {"window_id": 101, "x": 10, "y": 20, "width": 400, "height": 300},
                    {"window_id": 303, "x": 410, "y": 20, "width": 400, "height": 300},
                ],
            },
        },
      },
      "scope_audit": {
        "scope_widened": False,
        "display_fallback_used": False,
      },
    },
]
evidence["session_snapshots"] = [{
    "session_id": session_id,
    "subject_ura": subject,
    "state": "closed",
    "terminal_receipt": {
        "receipt_type": "remoteapp.session.terminal.v1",
        "terminal": True,
        "terminal_event_type": "SESSION_CLOSED",
        "terminal_event_sequence": 90,
        "session_id": session_id,
        "subject_ura": subject,
        "reason_code": "user_cancelled",
        "binding_epoch": 2,
        "target_identity_epoch": 22,
        "target_geometry_revision": 2,
    },
}]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
"$SCRIPT" --run --evidence-json "$OUT_DIR/application-target-churn.json" \
  --out-dir "$OUT_DIR/application-target-churn" >/dev/null
grep -q '"status": "passed"' "$OUT_DIR/application-target-churn/report.json" || \
  fail "application target churn evidence must pass"

python3 - "$OUT_DIR/application-target-churn.json" "$OUT_DIR/churn-without-media.json" <<'PY'
import json
import sys
evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["application_target_churn"]["frames_rendered_after_rebind"] = 0
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/churn-without-media.json" \
    --out-dir "$OUT_DIR/churn-without-media" >/tmp/frontend-remoteapp-browser-lifecycle-churn-media.out 2>&1; then
  fail "verifier accepted application churn without post-rebind media"
fi
grep -q "must render media after rebind" /tmp/frontend-remoteapp-browser-lifecycle-churn-media.out || \
  fail "missing post-rebind media failure was not explicit"

python3 - "$OUT_DIR/application-target-churn.json" "$OUT_DIR/application-geometry-churn.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
churn = evidence["application_target_churn"]
churn.update({
    "proof_mode": "real_application_geometry_churn",
    "churn_mode": "geometry",
    "target_identity_epoch_after": churn["target_identity_epoch_before"],
    "resolved_window_ids_before": [101, 303],
    "resolved_window_ids_after": [101, 303],
    "target_events": ["TARGET_MOVED", "TARGET_RESIZED"],
})
initial_binding = evidence["target_execution_snapshots"][0]["target_binding"]
initial_binding["app_window_set"]["resolved_window_ids"] = [101, 303]
initial_binding["capture_proof"]["app_window_set"]["resolved_window_ids"] = [101, 303]
for surface in initial_binding["app_surface_layout"]["front_to_back_surfaces"]:
    if surface["window_id"] == 202:
        surface["window_id"] = 303
for surface in initial_binding["capture_proof"]["app_surface_layout"]["front_to_back_surfaces"]:
    if surface["window_id"] == 202:
        surface["window_id"] = 303
evidence["session_snapshots"][0]["terminal_receipt"]["target_identity_epoch"] = \
    churn["target_identity_epoch_after"]
for step in evidence["steps"]:
    if step["name"] == "media_presented_after_application_window_set_rebind":
        step["name"] = "media_presented_after_application_geometry_rebind"
        step["target_identity_epoch"] = churn["target_identity_epoch_after"]
    elif step["name"] == "input_applied_after_application_window_set_rebind":
        step["name"] = "input_applied_after_application_geometry_rebind"
    elif step["name"] == "application_window_set_rebound":
        step["name"] = "application_geometry_rebound"
        step["churn_mode"] = "geometry"
        step["target_identity_epoch_after"] = churn["target_identity_epoch_after"]
        step["target_events"] = churn["target_events"]
binding_id = "tb_geometry_2"
transport_epoch = 77
media_source_epoch = 2
evidence["target_lifecycle_events"] = []
for sequence, event_type in enumerate(churn["target_events"], start=31):
    evidence["target_lifecycle_events"].append({
        "source_ability": "remote_desktop.show_session",
        "session_id": churn["session_id"],
        "subject_ura": churn["selected_resource_ura"],
        "sequence": sequence,
        "event_type": event_type,
        "terminal": False,
        "binding_id": binding_id,
        "binding_epoch": churn["binding_epoch_after"],
        "transport_epoch": transport_epoch,
        "media_source_epoch": media_source_epoch,
        "target_identity_epoch": churn["target_identity_epoch_after"],
        "target_geometry_revision": churn["target_geometry_revision_after"],
        "payload": {
            "subject_ura": churn["selected_resource_ura"],
            "previous_binding_epoch": churn["binding_epoch_before"],
            "previous_target_identity_epoch": churn["target_identity_epoch_before"],
            "previous_target_geometry_revision": churn["target_geometry_revision_before"],
            "binding_epoch": churn["binding_epoch_after"],
            "target_identity_epoch": churn["target_identity_epoch_after"],
            "target_geometry_revision": churn["target_geometry_revision_after"],
            "target_binding": {
                "binding_id": binding_id,
                "binding_epoch": churn["binding_epoch_after"],
                "target_geometry_revision": churn["target_geometry_revision_after"],
                "app_window_set": {
                    "resolved_window_ids": churn["resolved_window_ids_after"],
                },
            },
        },
    })
for interaction in churn["input_after_rebind"]["interaction_sequence"]:
    interaction["applied_event"]["target_geometry_revision"] = churn["target_geometry_revision_after"]
churn["input_after_rebind"]["input_probe"] = {
    "source": "committed_application_surface_center",
    "window_id": churn["resolved_window_ids_after"][0],
}
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
"$SCRIPT" --run --evidence-json "$OUT_DIR/application-geometry-churn.json" \
  --out-dir "$OUT_DIR/application-geometry-churn" >/dev/null
grep -q '"status": "passed"' "$OUT_DIR/application-geometry-churn/report.json" || \
  fail "application geometry churn evidence must pass"

python3 - "$OUT_DIR/application-geometry-churn.json" "$OUT_DIR/geometry-churn-coded-size-as-native.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
proof = evidence["target_execution_snapshots"][-1]["target_binding"]["capture_proof"]
proof["native_width"] = 1280
proof["native_height"] = 720
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/geometry-churn-coded-size-as-native.json" \
    --out-dir "$OUT_DIR/geometry-churn-coded-size-as-native" \
    >/tmp/frontend-remoteapp-browser-lifecycle-coded-size-as-native.out 2>&1; then
  fail "verifier accepted coded presentation dimensions as Linux native capture proof"
fi
grep -q "native proof must match the exact surface-layout union" \
  /tmp/frontend-remoteapp-browser-lifecycle-coded-size-as-native.out || \
  fail "coded-as-native capture proof failure was not explicit"

python3 - "$OUT_DIR/application-geometry-churn.json" "$OUT_DIR/geometry-churn-without-resize-event.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["application_target_churn"]["target_events"] = ["TARGET_MOVED"]
evidence["target_lifecycle_events"] = evidence["target_lifecycle_events"][:1]
for step in evidence["steps"]:
    if step["name"] == "application_geometry_rebound":
        step["target_events"] = ["TARGET_MOVED"]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/geometry-churn-without-resize-event.json" \
    --out-dir "$OUT_DIR/geometry-churn-without-resize-event" \
    >/tmp/frontend-remoteapp-browser-lifecycle-geometry-events.out 2>&1; then
  fail "verifier accepted application geometry churn without TARGET_RESIZED"
fi
grep -q "must expose ordered TARGET_MOVED and TARGET_RESIZED events" \
  /tmp/frontend-remoteapp-browser-lifecycle-geometry-events.out || \
  fail "missing application geometry lifecycle event failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/window-target-churn.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["evidence_origin"] = "live_runner"
session_id = evidence["session_id"]
subject = evidence["selected_resource_ura"]
for key in ("transport_resume", "transport_snapshots", "transport_disconnect_observations"):
    evidence.pop(key, None)
resume_steps = {
    "transport_disconnected", "session_preserved_for_reconnect",
    "transport_reconnected", "watch_events_reestablished",
    "media_presented_after_resume", "input_control_after_resume",
}
evidence["steps"] = [step for step in evidence["steps"] if step["name"] not in resume_steps]
evidence["selected_target_kind"] = "window"
evidence["selected_target_snapshot"] = {
    "resource_ura": subject,
    "type": "window",
    "metadata": {
        "capture_target": "window",
        "platform": "linux",
        "backend": "xcap",
        "window_id": 4194334,
        "x": 80,
        "y": 100,
        "width": 2560,
        "height": 1664,
    },
}
binding_before = 1
binding_after = 2
media_before = 3
media_after = 4
transport_epoch = 77
identity_epoch = 5
geometry_before = 6
geometry_after = 7
verified_before = 1787332000200
verified_after = 1787332000300
native_before = (2560, 1664)
native_after = (2200, 1400)
frame_before = (1108, 720)
frame_after = (1130, 720)
window_id = 4194334

def snapshot(binding_epoch, media_epoch, geometry_revision, verified_at, native, bounds, observed_at):
    return {
        "observed_at_ms": observed_at,
        "source_ability": "remote_desktop.show_session",
        "session_id": session_id,
        "subject_ura": subject,
        "target_binding": {
            "subject_ura": subject,
            "target_kind": "window",
            "binding_epoch": binding_epoch,
            "media_source_epoch": media_epoch,
            "target_identity_epoch": identity_epoch,
            "target_geometry_revision": geometry_revision,
            "capture_scope": "WindowSurface",
            "bounds": {"x": bounds[0], "y": bounds[1], "width": bounds[2], "height": bounds[3]},
            "capture_proof": {
                "target_kind": "window",
                "backend": "xcap",
                "display_id": None,
                "window_id": window_id,
                "verified_at_ms": verified_at,
                "native_width": native[0],
                "native_height": native[1],
            },
        },
        "scope_audit": {
            "requested_target_kind": "window",
            "effective_target_kind": "window",
            "capture_surface": "WindowSurface",
            "scope_widened": False,
            "display_fallback_used": False,
        },
    }

initial_snapshot = snapshot(
    binding_before, media_before, geometry_before,
    verified_before, native_before, (80, 100, *native_before), 1787332000201)
rebound_snapshot = snapshot(
    binding_after, media_after, geometry_after,
    verified_after, native_after, (160, 240, *native_after), 1787332000301)
evidence["target_execution_snapshot"] = initial_snapshot
evidence["target_execution_snapshots"] = [initial_snapshot, rebound_snapshot]

event_types = ["TARGET_MOVED", "TARGET_RESIZED"]
event_sequences = [31, 32]
evidence["target_lifecycle_events"] = []
for sequence, event_type in zip(event_sequences, event_types):
    evidence["target_lifecycle_events"].append({
        "source_ability": "remote_desktop.show_session",
        "session_id": session_id,
        "subject_ura": subject,
        "sequence": sequence,
        "event_type": event_type,
        "terminal": False,
        "binding_epoch": binding_after,
        "media_source_epoch": media_after,
        "transport_epoch": transport_epoch,
        "target_identity_epoch": identity_epoch,
        "target_geometry_revision": geometry_after,
        "payload": {
            "subject_ura": subject,
            "previous_binding_epoch": binding_before,
            "previous_media_source_epoch": media_before,
            "previous_target_identity_epoch": identity_epoch,
            "previous_target_geometry_revision": geometry_before,
        },
    })

interactions = []
for sequence, frame_type, action in [
    (21, "pointer", "down"), (22, "pointer", "up"),
    (23, "key", "down"), (24, "key", "up"),
]:
    frame = {
        "type": frame_type,
        "action": action,
        "client_sequence": sequence,
        "target_focus_epoch": 8,
        "sent_at_ms": 1787332000400 + sequence,
    }
    if frame_type == "pointer":
        frame.update({
            "target_width": native_after[0],
            "target_height": native_after[1],
            "target_geometry_revision": geometry_after,
        })
    interactions.append({
        "client_sequence": sequence,
        "target_focus_epoch": 8,
        "target_geometry_revision": geometry_after,
        "latency_ms": 8,
        "submitted_frame": frame,
        "applied_event": {
            "event_type": "INPUT_FRAME_APPLIED",
            "session_id": session_id,
            "client_sequence": sequence,
            "target_focus_epoch": 8,
            "target_geometry_revision": geometry_after,
        },
    })
churn_input = {
    "result": "input_applied",
    "visible_status": "input scope target_local · pointer+keyboard",
    "client_sequence": 21,
    "target_focus_epoch": 8,
    "target_geometry_revision": geometry_after,
    "latency_ms": 8,
    "submitted_frame": interactions[0]["submitted_frame"],
    "applied_event": interactions[0]["applied_event"],
    "interaction_sequence": interactions,
    "input_probe": {"source": "target_center", "window_id": None},
}
summary = {
    "proof_mode": "real_window_geometry_capture_generation_churn",
    "churn_mode": "geometry",
    "selected_resource_ura": subject,
    "session_id": session_id,
    "binding_epoch_before": binding_before,
    "binding_epoch_after": binding_after,
    "media_source_epoch_before": media_before,
    "media_source_epoch_after": media_after,
    "transport_epoch_before": transport_epoch,
    "transport_epoch_after": transport_epoch,
    "target_identity_epoch_before": identity_epoch,
    "target_identity_epoch_after": identity_epoch,
    "target_geometry_revision_before": geometry_before,
    "target_geometry_revision_after": geometry_after,
    "capture_verified_at_ms_before": verified_before,
    "capture_verified_at_ms_after": verified_after,
    "window_id_before": window_id,
    "window_id_after": window_id,
    "native_width_before": native_before[0],
    "native_height_before": native_before[1],
    "native_width_after": native_after[0],
    "native_height_after": native_after[1],
    "presentation_max_width": 1280,
    "presentation_max_height": 720,
    "presentation_scale_mode": "native",
    "frame_width_before": frame_before[0],
    "frame_height_before": frame_before[1],
    "expected_frame_width_before": frame_before[0],
    "expected_frame_height_before": frame_before[1],
    "frame_width_after": frame_after[0],
    "frame_height_after": frame_after[1],
    "expected_frame_width_after": frame_after[0],
    "expected_frame_height_after": frame_after[1],
    "logical_input_width": native_after[0],
    "logical_input_height": native_after[1],
    "target_events": event_types,
    "target_event_sequences": event_sequences,
    "frames_rendered_after_rebind": 2,
    "input_after_rebind": churn_input,
    "scope_widened": False,
    "display_fallback_used": False,
}
evidence["window_target_churn"] = summary
ended_index = next(i for i, step in enumerate(evidence["steps"]) if step["name"] == "session_ended")
evidence["steps"][ended_index:ended_index] = [
    {
        "name": "media_presented_after_window_geometry_rebind",
        "status": "passed", "evidence_source": "browser_automation",
        "component_snapshot_only": False, "observed_at_ms": 1787332000120,
        "subject_ura": subject, "session_id": session_id,
        "binding_epoch": binding_after, "media_source_epoch": media_after,
        "capture_verified_at_ms": verified_after,
        "frame_presented": True, "media_element_visible": True,
        "frames_presented": 5, "frame_width": frame_after[0], "frame_height": frame_after[1],
        "expected_frame_width": frame_after[0], "expected_frame_height": frame_after[1],
    },
    {
        "name": "input_applied_after_window_geometry_rebind",
        "status": "passed", "evidence_source": "browser_automation",
        "component_snapshot_only": False, "observed_at_ms": 1787332000130,
        **churn_input,
    },
    {
        "name": "window_geometry_rebound",
        "status": "passed", "evidence_source": "browser_automation",
        "component_snapshot_only": False, "observed_at_ms": 1787332000140,
        "subject_ura": subject, "session_id": session_id,
        **{key: value for key, value in summary.items()
           if key not in {"proof_mode", "churn_mode", "selected_resource_ura",
                          "session_id", "input_after_rebind"}},
        "input_applied_after_rebind": True,
    },
]
evidence["session_snapshots"] = [{
    "source_ability": "remote_desktop.end_session",
    "session_id": session_id,
    "subject_ura": subject,
    "state": "closed",
    "terminal_receipt": {
        "receipt_type": "remoteapp.session.terminal.v1",
        "terminal": True,
        "terminal_event_type": "SESSION_CLOSED",
        "terminal_event_sequence": 90,
        "session_id": session_id,
        "subject_ura": subject,
        "subject_type": "window",
        "reason_code": "user_cancelled",
        "binding_epoch": binding_after,
        "media_source_epoch": media_after,
        "target_identity_epoch": identity_epoch,
        "target_geometry_revision": geometry_after,
    },
}]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
"$SCRIPT" --run --evidence-json "$OUT_DIR/window-target-churn.json" \
  --out-dir "$OUT_DIR/window-target-churn" >/dev/null
grep -q '"status": "passed"' "$OUT_DIR/window-target-churn/report.json" || \
  fail "window capture-generation churn evidence must pass"

python3 - "$OUT_DIR/window-target-churn.json" "$OUT_DIR/window-host-input-effects.json" <<'PY'
import copy
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["host_input_effects_required"] = True
window_id = evidence["selected_target_snapshot"]["metadata"]["window_id"]
title = evidence["selected_target_snapshot"]["metadata"].get("title", "Synthetic Window")
observer_pid = 4242
evidence["selected_target_snapshot"]["metadata"]["pid"] = observer_pid
observer_identity = {
    "instance_id": "synthetic-observer-instance",
    "pid": observer_pid,
    "process_start_ticks": 987654,
    "boot_id": "11111111-2222-3333-4444-555555555555",
    "display": ":99",
    "fixture_sha256": "a" * 64,
    "started_at_ms": 1787331990000,
    "event_source": "target_process_tk_x11_callbacks",
}

def decorate(input_evidence, baseline):
    selected_events = []
    correlations = []
    interactions = input_evidence["interaction_sequence"]
    for index, interaction in enumerate(interactions):
        frame = interaction["submitted_frame"]
        frame.setdefault("x", 120)
        frame.setdefault("y", 80)
        frame.setdefault("button", 0)
        frame.setdefault("key", "a")
        event_id = f"rdinp1_{interaction['client_sequence']:032x}"
        safety_release = False
        interaction["applied_event"].update({
            "input_event_id": event_id,
            "host_received_at_ms": frame["sent_at_ms"],
            "host_applied_at_ms": frame["sent_at_ms"] + (2 if index == 0 else 1),
            "sequence": 1000 + baseline + index,
            "transport_epoch": 77,
            "safety_release": safety_release,
        })
        guard = {
            "status": "passed",
            "session_id": evidence["session_id"],
            "subject_ura": evidence["selected_resource_ura"],
            "target_kind": "window",
            "window_id_exact": True,
            "expected_pid": observer_pid,
            "expected_process_instance_id": (
                f"linux:{observer_identity['boot_id']}:{observer_pid}:"
                f"{observer_identity['process_start_ticks']}"
            ),
            "atomicity": "x11_server_grab",
            "snapshot_started_at_ms": frame["sent_at_ms"] - 2,
            "guard_acquired_at_ms": frame["sent_at_ms"] - 2,
            "validated_at_ms": frame["sent_at_ms"] - 1,
            "injected_at_ms": frame["sent_at_ms"],
            "guard_released_at_ms": frame["sent_at_ms"],
            "target_focus_epoch": interaction["applied_event"]["target_focus_epoch"],
            "target_geometry_revision": interaction["applied_event"]["target_geometry_revision"],
        }
        if frame["type"] == "pointer":
            guard.update({
                "pointer_target_window_id": window_id,
                "pointer_occlusion_checked": True,
            })
            interaction["applied_event"]["pointer_position_applied"] = True
        interaction["applied_event"]["target_guard_validation"] = guard
        selected_events.append({
            "sequence": baseline + index + 1,
            "at_ms": frame["sent_at_ms"] + (1 if index == 0 else 2),
            "kind": "keyboard" if frame["type"] == "key" else "pointer",
            "action": frame["action"],
            "surface": "A",
            "native_window_id": window_id,
            "client_window_id": window_id - 1,
            "x": frame["x"],
            "y": frame["y"],
            "button": frame["button"] + 1 if frame["type"] == "pointer" else 0,
            "keysym": frame["key"] if frame["type"] == "key" else "??",
        })
        correlations.append({
            "observer_event_sequence": baseline + index + 1,
            "daemon_runtime_event_sequence": interaction["applied_event"]["sequence"],
            "daemon_input_event_id": event_id,
            "host_effect_offset_from_apply_ms": -1 if index == 0 else 1,
        })
    first_at_ms = min(interaction["submitted_frame"]["sent_at_ms"]
                      for interaction in interactions)
    selected_window = {
        "surface": "A",
        "title": title,
        "client_window_id": window_id - 1,
        "native_window_id": window_id,
        "x": 80,
        "y": 100,
        "width": input_evidence["interaction_sequence"][0]["submitted_frame"].get(
            "target_width", 480),
        "height": input_evidence["interaction_sequence"][0]["submitted_frame"].get(
            "target_height", 320),
        "viewable": True,
    }
    unrelated_window = {
        "surface": "B",
        "title": "Synthetic Unrelated Window",
        "client_window_id": window_id + 99,
        "native_window_id": window_id + 100,
        "x": 620,
        "y": 160,
        "width": 480,
        "height": 320,
        "viewable": True,
    }
    baseline_health = {
        "status": "healthy",
        "callback_error_count": 0,
        "event_count": baseline,
    }
    final_health = {
        "status": "healthy",
        "callback_error_count": 0,
        "event_count": baseline + 4,
    }
    input_evidence["applied_event"] = interactions[0]["applied_event"]
    input_evidence["host_input_effects"] = {
        "observer_schema": "easynet.remoteapp.linux-x11-sentinel.v1",
        "target_kind": "window",
        "observer_independence": {
            "proof_mode": "selected_target_process_x11_callback_log",
            "target_process_pid": observer_pid,
            "observer_process_pid": observer_pid,
            "target_pid_matches_observer": True,
            "stable_process_instance": True,
            "daemon_event_ids_absent_from_observer_log": True,
        },
        "observer_baseline": {
            "schema": "easynet.remoteapp.linux-x11-sentinel.v1",
            "observer_identity": observer_identity,
            "observed_at_ms": first_at_ms - 10,
            "tick": baseline + 10,
            "windows": [selected_window, unrelated_window],
            "observer_health": baseline_health,
        },
        "observer_final": {
            "schema": "easynet.remoteapp.linux-x11-sentinel.v1",
            "observer_identity": observer_identity,
            "observed_at_ms": interactions[-1]["submitted_frame"]["sent_at_ms"] + 10,
            "tick": baseline + 20,
            "windows": [selected_window, unrelated_window],
            "observer_health": final_health,
        },
        "observer_health": final_health,
        "baseline_event_count": baseline,
        "final_event_count": baseline + 4,
        "selected_surface": "A",
        "selected_native_window_id": window_id,
        "selected_native_window_ids": [window_id],
        "selected_title": title,
        "selected_events": selected_events,
        "event_correlations": correlations,
        "unexpected_input_event_count": 0,
        "exact_target_effect_observed": True,
        "exact_window_effect_observed": True,
    }

initial = next(step for step in evidence["steps"]
               if step["name"] == "input_control_attempted_or_policy_blocked")
if "interaction_sequence" not in initial:
    observed_at_ms = initial["observed_at_ms"]
    initial.clear()
    initial.update({
        "name": "input_control_attempted_or_policy_blocked",
        "status": "passed",
        "evidence_source": "browser_automation",
        "component_snapshot_only": False,
        "observed_at_ms": observed_at_ms,
        **copy.deepcopy(evidence["window_target_churn"]["input_after_rebind"]),
    })
decorate(initial, 0)
decorate(evidence["window_target_churn"]["input_after_rebind"], 4)
rebound = next(step for step in evidence["steps"]
               if step["name"] == "input_applied_after_window_geometry_rebind")
rebound.update(evidence["window_target_churn"]["input_after_rebind"])
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
"$SCRIPT" --run --evidence-json "$OUT_DIR/window-host-input-effects.json" \
  --out-dir "$OUT_DIR/window-host-input-effects" >/dev/null
grep -q '"host_input_effects_verified": true' \
  "$OUT_DIR/window-host-input-effects/report.json" || {
  sed -n '1,240p' "$OUT_DIR/window-host-input-effects/report.json" >&2
  fail "independent exact-window host input effects must pass"
}

window_mutation() {
  local name="$1"
  local python_body="$2"
  local expected_error="$3"
  python3 - "$OUT_DIR/window-target-churn.json" "$OUT_DIR/$name.json" "$python_body" <<'PY'
import json
import sys

source, destination, body = sys.argv[1:]
evidence = json.load(open(source, encoding="utf-8"))
exec(body, {"evidence": evidence})
json.dump(evidence, open(destination, "w", encoding="utf-8"), indent=2)
PY
  if "$SCRIPT" --run --evidence-json "$OUT_DIR/$name.json" \
      --out-dir "$OUT_DIR/$name" >"/tmp/frontend-remoteapp-browser-$name.out" 2>&1; then
    fail "verifier accepted invalid window churn mutation: $name"
  fi
  grep -q "$expected_error" "/tmp/frontend-remoteapp-browser-$name.out" || \
    fail "window churn mutation did not expose expected error: $name"
}

host_effect_mutation() {
  local name="$1"
  local python_body="$2"
  local expected_error="$3"
  python3 - "$OUT_DIR/window-host-input-effects.json" "$OUT_DIR/$name.json" "$python_body" <<'PY'
import json
import sys

source, destination, body = sys.argv[1:]
evidence = json.load(open(source, encoding="utf-8"))
exec(body, {"evidence": evidence})
json.dump(evidence, open(destination, "w", encoding="utf-8"), indent=2)
PY
  if "$SCRIPT" --run --evidence-json "$OUT_DIR/$name.json" \
      --out-dir "$OUT_DIR/$name" >"/tmp/frontend-remoteapp-browser-$name.out" 2>&1; then
    fail "verifier accepted invalid host input effects: $name"
  fi
  grep -q "$expected_error" "/tmp/frontend-remoteapp-browser-$name.out" || \
    fail "host-effect mutation did not expose expected error: $name"
}

host_effect_mutation "host-input-unrelated-effect" \
  'evidence["window_target_churn"]["input_after_rebind"]["host_input_effects"]["unexpected_input_event_count"] = 1' \
  "must prove zero unexpected non-motion input effects"
host_effect_mutation "host-input-missing-event-id" \
  'del evidence["window_target_churn"]["input_after_rebind"]["interaction_sequence"][0]["applied_event"]["input_event_id"]' \
  "must bind a canonical daemon input_event_id"
host_effect_mutation "host-input-id-grafted-into-observer" \
  'effects = evidence["window_target_churn"]["input_after_rebind"]["host_input_effects"]; effects["selected_events"][0]["input_event_id"] = effects["event_correlations"][0]["daemon_input_event_id"]' \
  "raw observer event must not contain daemon input_event_id"
host_effect_mutation "host-input-observer-instance-replaced" \
  'evidence["window_target_churn"]["input_after_rebind"]["host_input_effects"]["observer_final"]["observer_identity"]["instance_id"] = "replacement"' \
  "baseline/final must bind one observer process instance"
host_effect_mutation "host-input-no-unrelated-window" \
  'evidence["window_target_churn"]["input_after_rebind"]["host_input_effects"]["observer_final"]["windows"] = evidence["window_target_churn"]["input_after_rebind"]["host_input_effects"]["observer_final"]["windows"][:1]' \
  "must retain one stable viewable unrelated Window"
host_effect_mutation "host-input-outside-correlation-window" \
  'effects = evidence["window_target_churn"]["input_after_rebind"]["host_input_effects"]; effects["selected_events"][0]["at_ms"] += 252; effects["event_correlations"][0]["host_effect_offset_from_apply_ms"] += 252' \
  "must fall inside the 250ms host-effect correlation window"
host_effect_mutation "host-input-before-daemon-receipt" \
  'effects = evidence["window_target_churn"]["input_after_rebind"]["host_input_effects"]; applied = evidence["window_target_churn"]["input_after_rebind"]["interaction_sequence"][0]["applied_event"]; effects["selected_events"][0]["at_ms"] = applied["host_received_at_ms"] - 1; effects["event_correlations"][0]["host_effect_offset_from_apply_ms"] = effects["selected_events"][0]["at_ms"] - applied["host_applied_at_ms"]' \
  "must follow submission and daemon receipt"
host_effect_mutation "host-input-runtime-sequence-reused" \
  'seq = evidence["window_target_churn"]["input_after_rebind"]["interaction_sequence"][0]["applied_event"]["sequence"]; evidence["window_target_churn"]["input_after_rebind"]["interaction_sequence"][1]["applied_event"]["sequence"] = seq; evidence["window_target_churn"]["input_after_rebind"]["host_input_effects"]["event_correlations"][1]["daemon_runtime_event_sequence"] = seq' \
  "daemon applied-event sequences must strictly advance"
host_effect_mutation "host-input-transport-generation-changed" \
  'evidence["window_target_churn"]["input_after_rebind"]["interaction_sequence"][3]["applied_event"]["transport_epoch"] += 1' \
  "correlated input must remain on one positive transport epoch"
host_effect_mutation "host-input-guard-wrong-subject" \
  'evidence["window_target_churn"]["input_after_rebind"]["interaction_sequence"][0]["applied_event"]["target_guard_validation"]["subject_ura"] += ".other"' \
  "must retain exact fresh window target-guard proof"
host_effect_mutation "host-input-emergency-release-in-happy-path" \
  'applied = evidence["window_target_churn"]["input_after_rebind"]["interaction_sequence"][1]["applied_event"]; applied["safety_release"] = True; applied["safety_release_reason"] = "target_input_guard_focus_mismatch"; applied["pointer_position_applied"] = False; applied.pop("target_guard_validation", None)' \
  "happy-path input must use normal guarded admission"
host_effect_mutation "host-input-observer-unhealthy" \
  'evidence["window_target_churn"]["input_after_rebind"]["host_input_effects"]["observer_health"]["callback_error_count"] = 1' \
  "host observer must remain healthy"
host_effect_mutation "host-input-wrong-window" \
  'evidence["window_target_churn"]["input_after_rebind"]["host_input_effects"]["selected_native_window_id"] += 1' \
  "must bind the selected native window id"

window_mutation "window-stale-media-epoch" \
  'evidence["target_execution_snapshots"][-1]["target_binding"]["media_source_epoch"] = 3' \
  "window geometry churn must expose the rebound capture generation"
window_mutation "window-stale-proof" \
  'evidence["target_execution_snapshots"][-1]["target_binding"]["capture_proof"]["verified_at_ms"] = 1787332000200' \
  "window snapshots must bind the stable native window and refreshed proof"
window_mutation "window-wrong-presentation" \
  'evidence["window_target_churn"]["frame_width_after"] = 2200' \
  "window rebound decoded media must match independently derived FitWithin/even presentation"
window_mutation "window-missing-resized" \
  'evidence["window_target_churn"]["target_events"] = ["TARGET_MOVED"]; evidence["window_target_churn"]["target_event_sequences"] = [31]; evidence["target_lifecycle_events"] = evidence["target_lifecycle_events"][:1]' \
  "must expose TARGET_RESIZED"
window_mutation "window-stale-input-revision" \
  'evidence["window_target_churn"]["input_after_rebind"]["interaction_sequence"][0]["applied_event"]["target_geometry_revision"] = 6' \
  "must bind the daemon applied event"
window_mutation "window-stale-input-dimensions" \
  'evidence["window_target_churn"]["input_after_rebind"]["interaction_sequence"][0]["submitted_frame"]["target_width"] = 1108' \
  "pointer frame must bind rebound logical dimensions"
window_mutation "window-scope-widened" \
  'evidence["target_execution_snapshots"][-1]["scope_audit"]["scope_widened"] = True' \
  "window rebound snapshot must preserve exact WindowSurface scope"
window_mutation "window-transport-replaced" \
  'evidence["window_target_churn"]["transport_epoch_after"] = 78' \
  "must preserve one transport epoch"

python3 - "$OUT_DIR/application-target.json" "$OUT_DIR/linux-application-target.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
metadata = evidence["selected_target_snapshot"]["metadata"]
metadata["platform"] = "linux"
metadata["discovery_scope"] = "process_window_set"
metadata.pop("display_scoped", None)
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
"$SCRIPT" --run --evidence-json "$OUT_DIR/linux-application-target.json" --out-dir "$OUT_DIR/linux-application-target" >/dev/null
grep -q '"status": "passed"' "$OUT_DIR/linux-application-target/report.json" || \
  fail "exact Linux process-window-set application evidence must pass"

python3 - "$OUT_DIR/application-target.json" "$OUT_DIR/invalid-application-scope.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["selected_target_snapshot"]["metadata"]["discovery_scope"] = "display"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/invalid-application-scope.json" --out-dir "$OUT_DIR/invalid-application-scope" >/tmp/frontend-remoteapp-browser-lifecycle-invalid-application-scope.out 2>&1; then
  fail "verifier accepted a non-application discovery scope"
fi
grep -q "discovery_scope must be an exact application window set" /tmp/frontend-remoteapp-browser-lifecycle-invalid-application-scope.out || \
  fail "invalid application discovery-scope rejection was not explicit"

python3 - "$OUT_DIR/application-target.json" "$OUT_DIR/display-widened-application.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["selected_target_snapshot"]["metadata"]["display_scoped"] = True
evidence["selected_target_snapshot"]["metadata"]["display_id"] = 1
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/display-widened-application.json" --out-dir "$OUT_DIR/display-widened-application" >/tmp/frontend-remoteapp-browser-lifecycle-display-widened.out 2>&1; then
  fail "verifier accepted a display-scoped application target"
fi
grep -q "application target must not be display scoped" /tmp/frontend-remoteapp-browser-lifecycle-display-widened.out || \
  fail "display-widened application rejection was not explicit"

python3 - "$OUT_DIR/application-target.json" "$OUT_DIR/execution-scope-widened-application.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["target_execution_snapshot"]["scope_audit"]["scope_widened"] = True
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/execution-scope-widened-application.json" --out-dir "$OUT_DIR/execution-scope-widened-application" >/tmp/frontend-remoteapp-browser-lifecycle-execution-scope-widened.out 2>&1; then
  fail "verifier accepted widened Runtime application execution"
fi
grep -q "application execution scope must not widen" /tmp/frontend-remoteapp-browser-lifecycle-execution-scope-widened.out || \
  fail "Runtime application scope-widening rejection was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/terminal-crash-replay.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["evidence_origin"] = "live_runner"
session_id = evidence["session_id"]
subject = evidence["selected_resource_ura"]
for key in ("transport_resume", "transport_snapshots", "transport_disconnect_observations"):
    evidence.pop(key, None)
resume_steps = {
    "transport_disconnected",
    "session_preserved_for_reconnect",
    "transport_reconnected",
    "watch_events_reestablished",
    "media_presented_after_resume",
    "input_control_after_resume",
}
evidence["steps"] = [step for step in evidence["steps"] if step["name"] not in resume_steps]
ended_index = next(i for i, step in enumerate(evidence["steps"]) if step["name"] == "session_ended")
evidence["steps"][ended_index:ended_index] = [
    {
        "name": "terminal_crash_armed",
        "status": "passed",
        "evidence_source": "browser_automation",
        "component_snapshot_only": False,
        "observed_at_ms": 1787332000170,
        "subject_ura": subject,
        "session_id": session_id,
        "fault": "crash_after_terminal_promotion",
    },
    {
        "name": "terminal_crash_observed",
        "status": "passed",
        "evidence_source": "browser_automation",
        "component_snapshot_only": False,
        "observed_at_ms": 1787332000175,
        "subject_ura": subject,
        "session_id": session_id,
        "device_online": "false",
        "state_code": "C440",
    },
]
for step in evidence["steps"]:
    if step["name"] == "session_ended":
        step["response_lost_to_daemon_crash"] = True
    if step["name"] == "terminal_receipt_visible":
        step["reason_code"] = "caller_ended"
        step["recovered_through"] = "remote_desktop.show_session"
evidence["terminal_crash_replay"] = {
    "proof_mode": "real_browser_terminal_promotion_crash_replay",
    "session_id": session_id,
    "subject_ura": subject,
    "same_public_session": True,
    "end_session_request_observed": True,
    "end_session_request_observed_at_ms": 1787332000171,
    "response_lost_to_daemon_crash": True,
    "device_offline_observed": True,
    "device_online_after_restart": True,
    "show_session_replayed_terminal": True,
    "terminal_replayed_at_ms": 1787332000185,
    "show_session_count_before": 1,
    "show_session_count_after": 2,
    "terminal": True,
    "reason_code": "caller_ended",
}
evidence["device_state_snapshots"] = [
    {"state_code": "J700"},
    {"state_code": "C440"},
    {"state_code": "J700"},
]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
"$SCRIPT" --run --evidence-json "$OUT_DIR/terminal-crash-replay.json" --out-dir "$OUT_DIR/terminal-crash-replay" >/dev/null
grep -q '"status": "passed"' "$OUT_DIR/terminal-crash-replay/report.json" || \
  fail "terminal promotion crash replay evidence must pass"

python3 - "$OUT_DIR/terminal-crash-replay.json" "$OUT_DIR/terminal-crash-no-show.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["terminal_crash_replay"]["show_session_count_after"] = 1
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/terminal-crash-no-show.json" --out-dir "$OUT_DIR/terminal-crash-no-show" >/tmp/frontend-remoteapp-browser-lifecycle-terminal-crash-no-show.out 2>&1; then
  fail "verifier accepted terminal crash recovery without a new show_session"
fi
grep -q "must prove a new public show_session observation" /tmp/frontend-remoteapp-browser-lifecycle-terminal-crash-no-show.out || \
  fail "missing terminal replay show_session failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/target-monitor-worker.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["evidence_origin"] = "live_runner"
for key in ("transport_resume", "transport_snapshots", "transport_disconnect_observations"):
    evidence.pop(key, None)
resume_steps = {
    "transport_disconnected",
    "session_preserved_for_reconnect",
    "transport_reconnected",
    "watch_events_reestablished",
    "media_presented_after_resume",
    "input_control_after_resume",
}
evidence["steps"] = [step for step in evidence["steps"] if step["name"] not in resume_steps]
session_id = evidence["session_id"]
subject = evidence["selected_resource_ura"]
ended_index = next(i for i, step in enumerate(evidence["steps"]) if step["name"] == "session_ended")
input_at = next(step["observed_at_ms"] for step in evidence["steps"]
                if step["name"] == "input_control_attempted_or_policy_blocked")
for step in evidence["steps"][ended_index:]:
    step["observed_at_ms"] += 100
worker_steps = [
    {
        "name": "target_monitor_crash_armed",
        "status": "passed",
        "evidence_source": "browser_automation",
        "component_snapshot_only": False,
        "observed_at_ms": input_at + 10,
        "subject_ura": subject,
        "session_id": session_id,
        "fault": "crash_target_monitor_generation",
    },
    {
        "name": "target_monitor_recovery_media_presented",
        "status": "passed",
        "evidence_source": "browser_automation",
        "component_snapshot_only": False,
        "observed_at_ms": input_at + 20,
        "frames_presented": 9,
        "frame_width": 1280,
        "frame_height": 720,
        "first_rendered_frame_at_ms": input_at + 20,
    },
    {
        "name": "target_monitor_recovered",
        "status": "passed",
        "evidence_source": "browser_automation",
        "component_snapshot_only": False,
        "observed_at_ms": input_at + 30,
        "subject_ura": subject,
        "session_id": session_id,
        "worker_events": ["PLUGIN_WORKER_CRASHED", "PLUGIN_WORKER_RESTARTED", "TARGET_MONITOR_RESTARTED"],
    },
]
evidence["steps"][ended_index:ended_index] = worker_steps
worker_event_records = [
    {
        "sequence": 20,
        "event_type": "PLUGIN_WORKER_CRASHED",
        "payload": {
            "component": "target_monitor",
            "failed_generation": 4,
        },
    },
    {
        "sequence": 21,
        "event_type": "PLUGIN_WORKER_RESTARTED",
        "payload": {
            "component": "target_monitor",
            "failed_generation": 4,
            "restarted_generation": 5,
        },
    },
    {
        "sequence": 22,
        "event_type": "TARGET_MONITOR_RESTARTED",
        "payload": {
            "component": "target_monitor",
            "failed_generation": 4,
            "restarted_generation": 5,
        },
    },
]
evidence["target_monitor_worker_recovery"] = {
    "proof_mode": "real_browser_target_monitor_worker_recovery",
    "session_id": session_id,
    "subject_ura": subject,
    "same_public_session": True,
    "ordered_worker_events": ["PLUGIN_WORKER_CRASHED", "PLUGIN_WORKER_RESTARTED", "TARGET_MONITOR_RESTARTED"],
    "worker_event_records": worker_event_records,
    "daemon_transport_epoch_preserved": True,
    "target_binding_epoch_preserved": True,
    "media_source_epoch_preserved": True,
    "consent_epoch_preserved": True,
    "transport_epoch_before": 8,
    "transport_epoch_after": 8,
    "binding_epoch_before": 3,
    "binding_epoch_after": 3,
    "media_source_epoch_before": 4,
    "media_source_epoch_after": 4,
    "consent_epoch_before": 2,
    "consent_epoch_after": 2,
    "frames_rendered_after_worker_restart": 2,
    "first_frame_rendered_after_worker_restart_at_ms": input_at + 20,
    "new_consent_required": False,
    "frontend_status": "remote desktop target monitor recovered; existing session and media remain active",
}
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
"$SCRIPT" --run --evidence-json "$OUT_DIR/target-monitor-worker.json" --out-dir "$OUT_DIR/target-monitor-worker" >/dev/null
grep -q '"status": "passed"' "$OUT_DIR/target-monitor-worker/report.json" || \
  fail "target-monitor worker recovery evidence must pass"

python3 - "$OUT_DIR/target-monitor-worker.json" "$OUT_DIR/target-monitor-new-media-epoch.json" <<'PY'
import json
import sys
evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["target_monitor_worker_recovery"]["media_source_epoch_after"] += 1
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/target-monitor-new-media-epoch.json" \
    --out-dir "$OUT_DIR/target-monitor-new-media-epoch" >/tmp/frontend-remoteapp-browser-lifecycle-target-monitor-epoch.out 2>&1; then
  fail "verifier accepted a needless media-source replacement for target-monitor recovery"
fi
grep -q "must preserve media_source_epoch" /tmp/frontend-remoteapp-browser-lifecycle-target-monitor-epoch.out || \
  fail "target-monitor media-source preservation failure was not explicit"

python3 - "$OUT_DIR/target-monitor-worker.json" "$OUT_DIR/target-monitor-generation-regressed.json" <<'PY'
import json
import sys
evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["target_monitor_worker_recovery"]["worker_event_records"][1]["payload"]["restarted_generation"] = 4
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/target-monitor-generation-regressed.json" \
    --out-dir "$OUT_DIR/target-monitor-generation-regressed" >/tmp/frontend-remoteapp-browser-lifecycle-target-monitor-generation.out 2>&1; then
  fail "verifier accepted a non-increasing target-monitor generation"
fi
grep -q "replacement generation must increase" /tmp/frontend-remoteapp-browser-lifecycle-target-monitor-generation.out || \
  fail "target-monitor generation failure was not explicit"

echo "test_frontend_remoteapp_browser_lifecycle_e2e: ok"
