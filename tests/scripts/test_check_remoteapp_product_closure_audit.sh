#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-product-closure-audit.sh"

fail() {
  printf 'test_check_remoteapp_product_closure_audit: %s\n' "$1" >&2
  exit 1
}

"$SCRIPT"

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT
mkdir -p \
  "$SB/docs/design" \
  "$SB/pr/20260822-remoteapp-product-closure" \
  "$SB/tests/scripts" \
  "$SB/tools/scripts" \
  "$SB/src/cli/commands" \
  "$SB/src/daemon/plugins" \
  "$SB/src/daemon/ability/builtins" \
  "$SB/plugins/remote-desktop/src/media" \
  "$SB/plugins/remote-desktop/src/transport" \
  "$SB/plugins/remote-desktop/src/handlers" \
  "$SB/plugins/remote-desktop/media-host/src" \
  "$SB/plugins/remote-desktop/native-protocol/src" \
  "$SB/plugins/remote-desktop/abilities"
cp "$SCRIPT" "$SB/tools/scripts/check-remoteapp-product-closure-audit.sh"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-device-product-smoke.sh" "$SB/tools/scripts/remoteapp-cross-device-product-smoke.sh"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh" "$SB/tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh"
cp "$REPO_ROOT/tools/scripts/remoteapp-product-completion-e2e.sh" "$SB/tools/scripts/remoteapp-product-completion-e2e.sh"
cp "$REPO_ROOT/tools/scripts/remoteapp-product-finalize.py" "$SB/tools/scripts/remoteapp-product-finalize.py"
cp "$REPO_ROOT/tools/scripts/remoteapp-evidence-provenance.py" "$SB/tools/scripts/remoteapp-evidence-provenance.py"
cp "$REPO_ROOT/tools/scripts/remoteapp-attestation-trust.py" "$SB/tools/scripts/remoteapp-attestation-trust.py"
cp "$REPO_ROOT/src/cli/commands/receipt_verification.rs" "$SB/src/cli/commands/receipt_verification.rs"
cp "$REPO_ROOT/tools/scripts/check-remoteapp-main-crate-implementation-tests.sh" "$SB/tools/scripts/check-remoteapp-main-crate-implementation-tests.sh"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-platform-capture-e2e.sh" "$SB/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"
cp "$REPO_ROOT/tools/scripts/remoteapp-input-injection-e2e.sh" "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-target-input-e2e.sh" "$SB/tools/scripts/host-remoteapp-target-input-e2e.sh"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/host-remoteapp-media-adaptation-e2e.sh"
cp "$REPO_ROOT/tools/scripts/remoteapp-multi-window-tracking-e2e.sh" "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"
cp "$REPO_ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh" "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-direct-e2e.sh" "$SB/tools/scripts/host-remoteapp-direct-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-turn-relay-e2e.sh" "$SB/tools/scripts/host-remoteapp-turn-relay-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-easynet-relay-e2e.sh" "$SB/tools/scripts/host-remoteapp-easynet-relay-e2e.sh"
cp "$REPO_ROOT/tools/scripts/verify-remoteapp-relay-refresh.py" "$SB/tools/scripts/verify-remoteapp-relay-refresh.py"
cp "$REPO_ROOT/tests/scripts/test_host_remoteapp_easynet_relay_e2e.sh" "$SB/tests/scripts/test_host_remoteapp_easynet_relay_e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-stun-srflx-e2e.sh" "$SB/tools/scripts/host-remoteapp-stun-srflx-e2e.sh"
cp "$REPO_ROOT/tools/scripts/remoteapp-stun-binding-server.py" "$SB/tools/scripts/remoteapp-stun-binding-server.py"
cp "$REPO_ROOT/tools/scripts/project-remoteapp-network-scenario.py" "$SB/tools/scripts/project-remoteapp-network-scenario.py"
cp "$REPO_ROOT/tools/scripts/frontend-remoteapp-product-flow-e2e.sh" "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
cp "$REPO_ROOT/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh" "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-permission-subject-e2e.sh" "$SB/tools/scripts/host-remoteapp-permission-subject-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh" "$SB/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh"
cp "$REPO_ROOT/tools/scripts/remoteapp-select-live-target.py" "$SB/tools/scripts/remoteapp-select-live-target.py"

cp "$REPO_ROOT/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" "$SB/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-decoded-frame-probe.sh" "$SB/tools/scripts/host-remoteapp-decoded-frame-probe.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh" "$SB/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-session-timeout-e2e.sh" "$SB/tools/scripts/host-remoteapp-session-timeout-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-session-cancel-e2e.sh" "$SB/tools/scripts/host-remoteapp-session-cancel-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-permission-revoke-e2e.sh" "$SB/tools/scripts/host-remoteapp-permission-revoke-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-session-resume-e2e.sh" "$SB/tools/scripts/host-remoteapp-session-resume-e2e.sh"
cp "$REPO_ROOT/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh" "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-target-monitor-worker-recovery-e2e.sh" "$SB/tools/scripts/host-remoteapp-target-monitor-worker-recovery-e2e.sh"
cp "$REPO_ROOT/tests/scripts/test_host_remoteapp_target_monitor_worker_recovery_e2e.sh" "$SB/tests/scripts/test_host_remoteapp_target_monitor_worker_recovery_e2e.sh"
cp "$REPO_ROOT/tools/scripts/remoteapp-lifecycle-harness-lib.sh" "$SB/tools/scripts/remoteapp-lifecycle-harness-lib.sh"
cp "$REPO_ROOT/docs/design/remoteapp-targeted-session-spec.md" "$SB/docs/design/remoteapp-targeted-session-spec.md"
cp "$REPO_ROOT/docs/design/remoteapp-product-readiness-audit-2026-08-22.md" "$SB/docs/design/remoteapp-product-readiness-audit-2026-08-22.md"
cp "$REPO_ROOT/docs/design/remoteapp-product-readiness-matrix.json" "$SB/docs/design/remoteapp-product-readiness-matrix.json"
cp "$REPO_ROOT/pr/20260822-remoteapp-product-closure/02-evidence-audit.md" "$SB/pr/20260822-remoteapp-product-closure/02-evidence-audit.md"
cp "$REPO_ROOT/plugins/remote-desktop/src/session.rs" "$SB/plugins/remote-desktop/src/session.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/session_recovery.rs" "$SB/plugins/remote-desktop/src/session_recovery.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/session_state.rs" "$SB/plugins/remote-desktop/src/session_state.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/session_store.rs" "$SB/plugins/remote-desktop/src/session_store.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/session_transport_state.rs" "$SB/plugins/remote-desktop/src/session_transport_state.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/session_lifecycle.rs" "$SB/plugins/remote-desktop/src/session_lifecycle.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/runtime.rs" "$SB/plugins/remote-desktop/src/runtime.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/relay_lease.rs" "$SB/plugins/remote-desktop/src/relay_lease.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/lease_monitor.rs" "$SB/plugins/remote-desktop/src/lease_monitor.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/embedded.rs" "$SB/plugins/remote-desktop/src/embedded.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/view_transport.rs" "$SB/plugins/remote-desktop/src/view_transport.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/view.rs" "$SB/plugins/remote-desktop/src/view.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/event_log.rs" "$SB/plugins/remote-desktop/src/event_log.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/session_events.rs" "$SB/plugins/remote-desktop/src/session_events.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/target.rs" "$SB/plugins/remote-desktop/src/target.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/target_tracking.rs" "$SB/plugins/remote-desktop/src/target_tracking.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/target_monitor.rs" "$SB/plugins/remote-desktop/src/target_monitor.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/target_snapshot.rs" "$SB/plugins/remote-desktop/src/target_snapshot.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/input.rs" "$SB/plugins/remote-desktop/src/input.rs"
cp "$REPO_ROOT/plugins/remote-desktop/plugin.toml" "$SB/plugins/remote-desktop/plugin.toml"
cp "$REPO_ROOT/plugins/remote-desktop/src/registration.rs" "$SB/plugins/remote-desktop/src/registration.rs"
cp "$REPO_ROOT/src/daemon/plugins/surface.rs" "$SB/src/daemon/plugins/surface.rs"
cp "$REPO_ROOT/src/daemon/plugins/remoteapp_relay.rs" "$SB/src/daemon/plugins/remoteapp_relay.rs"
cp "$REPO_ROOT/src/daemon/plugins/mod.rs" "$SB/src/daemon/plugins/mod.rs"
cp "$REPO_ROOT/src/daemon/ability/builtins/real_invoke_tests.rs" "$SB/src/daemon/ability/builtins/real_invoke_tests.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/media/mod.rs" "$SB/plugins/remote-desktop/src/media/mod.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/media/host_audio_capability.rs" "$SB/plugins/remote-desktop/src/media/host_audio_capability.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/handlers/mod.rs" "$SB/plugins/remote-desktop/src/handlers/mod.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/handlers/create_session.rs" "$SB/plugins/remote-desktop/src/handlers/create_session.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/handlers/refresh_lease.rs" "$SB/plugins/remote-desktop/src/handlers/refresh_lease.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/handlers/show_session.rs" "$SB/plugins/remote-desktop/src/handlers/show_session.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/handlers/end_session.rs" "$SB/plugins/remote-desktop/src/handlers/end_session.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/handlers/report_client_state.rs" "$SB/plugins/remote-desktop/src/handlers/report_client_state.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/schema.rs" "$SB/plugins/remote-desktop/src/schema.rs"
cp "$REPO_ROOT/plugins/remote-desktop/abilities/remote_desktop.report_client_state.ability.toml" "$SB/plugins/remote-desktop/abilities/remote_desktop.report_client_state.ability.toml"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs" "$SB/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/webrtc_encoded_audio.rs" "$SB/plugins/remote-desktop/src/transport/webrtc_encoded_audio.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/webrtc_endpoint.rs" "$SB/plugins/remote-desktop/src/transport/webrtc_endpoint.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/webrtc.rs" "$SB/plugins/remote-desktop/src/transport/webrtc.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/terminal.rs" "$SB/plugins/remote-desktop/src/transport/terminal.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/invoke_bidi.rs" "$SB/plugins/remote-desktop/src/invoke_bidi.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/native_host_process.rs" "$SB/plugins/remote-desktop/src/native_host_process.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/handlers/attach.rs" "$SB/plugins/remote-desktop/src/handlers/attach.rs"
cp "$REPO_ROOT/plugins/remote-desktop/media-host/src/lib.rs" "$SB/plugins/remote-desktop/media-host/src/lib.rs"
cp "$REPO_ROOT/plugins/remote-desktop/media-host/src/macos_sck.rs" "$SB/plugins/remote-desktop/media-host/src/macos_sck.rs"
cp "$REPO_ROOT/plugins/remote-desktop/media-host/src/macos_audio.rs" "$SB/plugins/remote-desktop/media-host/src/macos_audio.rs"
cp "$REPO_ROOT/plugins/remote-desktop/native-protocol/src/media_session.rs" "$SB/plugins/remote-desktop/native-protocol/src/media_session.rs"
cp "$REPO_ROOT/plugins/remote-desktop/native-protocol/src/shared_media_lane.rs" "$SB/plugins/remote-desktop/native-protocol/src/shared_media_lane.rs"

perl -0pi -e 's#"media_pipeline_id"#"pipeline_label"#g' \
  "$SB/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-hosted-pipeline-id.out 2>&1; then
  fail "checker accepted hosted media telemetry without product pipeline identity"
fi
grep -q "hosted WebRTC media stats must bind the stable product media pipeline id" /tmp/check-remoteapp-product-closure-hosted-pipeline-id.out || \
  fail "expected hosted media pipeline identity failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs" "$SB/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs"

perl -0pi -e 's#let frame = Bytes::from_owner\(lease\);#let frame = Bytes::copy_from_slice(lease.as_ref());#g' \
  "$SB/plugins/remote-desktop/src/native_host_process.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-shared-owner.out 2>&1; then
  fail "checker accepted a daemon whole-payload copy before WebRTC"
fi
grep -q "daemon media ingress must preserve mapped payload ownership" /tmp/check-remoteapp-product-closure-shared-owner.out || \
  fail "expected shared media ownership failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/native_host_process.rs" "$SB/plugins/remote-desktop/src/native_host_process.rs"

touch "$SB/plugins/remote-desktop/src/transport/webrtc_native_media.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-obsolete-media.out 2>&1; then
  fail "checker accepted obsolete daemon-local native media beside media-host"
fi
grep -q "obsolete daemon-local media implementation remains" /tmp/check-remoteapp-product-closure-obsolete-media.out || \
  fail "expected obsolete daemon-local media failure"
rm "$SB/plugins/remote-desktop/src/transport/webrtc_native_media.rs"

perl -0pi -e 's#run_step host-target-picker-freshness-application#application_target_freshness_removed#g' \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-application-picker-step.out 2>&1; then
  fail "checker accepted frontend product flow without application target-picker freshness"
fi
grep -q "distinct application target-picker freshness step" /tmp/check-remoteapp-product-closure-application-picker-step.out || \
  fail "expected missing application target-picker freshness failure"
cp "$REPO_ROOT/tools/scripts/frontend-remoteapp-product-flow-e2e.sh" "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"

perl -0pi -e 's#application_identity_plus_owner_pid_and_window_set#application_identity_only#g' \
  "$SB/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-application-picker-identity.out 2>&1; then
  fail "checker accepted application target freshness without owner/window-set identity"
fi
grep -q "application identity to owner process and exact window set" /tmp/check-remoteapp-product-closure-application-picker-identity.out || \
  fail "expected incomplete application target identity failure"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh" "$SB/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh"

perl -0pi -e 's#not resource_ura and target_pid is None and hint#hint#g' \
  "$SB/tools/scripts/remoteapp-select-live-target.py"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-target-selector.out 2>&1; then
  fail "checker accepted diagnostic picker labels as authoritative target identity"
fi
grep -q "diagnostic labels out of authoritative" /tmp/check-remoteapp-product-closure-target-selector.out || \
  fail "expected authoritative target selector identity failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-select-live-target.py" "$SB/tools/scripts/remoteapp-select-live-target.py"

perl -0pi -e 's#prepare_completion_signing_material#prepare_arbitrary_json#g' \
  "$SB/tools/scripts/remoteapp-product-finalize.py"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-signing-material.out 2>&1; then
  fail "checker accepted a completion authority without constrained signing material"
fi
grep -q "constrained DSSE signing material" /tmp/check-remoteapp-product-closure-signing-material.out || \
  fail "expected constrained completion signing-material failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-product-finalize.py" "$SB/tools/scripts/remoteapp-product-finalize.py"

perl -0pi -e 's#REQUIRED_PRODUCT_DOMAIN_IDS#OPTIONAL_PRODUCT_DOMAIN_IDS#g' \
  "$SB/tools/scripts/remoteapp-product-finalize.py"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-completion-domain-set.out 2>&1; then
  fail "checker accepted completion signing without an independently pinned domain set"
fi
grep -q "independently pin the complete product domain set" /tmp/check-remoteapp-product-closure-completion-domain-set.out || \
  fail "expected independent completion domain-set failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-product-finalize.py" "$SB/tools/scripts/remoteapp-product-finalize.py"

perl -0pi -e 's#next_quarantine_projection_at#next_quarantine_projection_time_removed#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-settlement-pacing.out 2>&1; then
  fail "checker accepted a quarantine projector without exact deadline scheduling"
fi
grep -q "sleep until the exact next projection attempt" /tmp/check-remoteapp-product-closure-settlement-pacing.out || \
  fail "expected settlement executor pacing failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#delayed_pending_job_is_not_polled_before_its_ready_time#delayed_pending_job_is_polled_before_its_ready_time#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-delayed-poll.out 2>&1; then
  fail "checker accepted persistence backoff implemented as empty polling"
fi
grep -q "persistence backoff does not become empty polling" /tmp/check-remoteapp-product-closure-delayed-poll.out || \
  fail "expected delayed settlement polling failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#dropped_reservation_before_terminal_remains_manager_visible#dropped_reservation_before_terminal_becomes_invisible#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-drop-first-reservation.out 2>&1; then
  fail "checker accepted a drop-first setup reservation visibility regression"
fi
grep -q "dropped setup reservations must remain visible" /tmp/check-remoteapp-product-closure-drop-first-reservation.out || \
  fail "expected drop-first reservation visibility failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#settlement_cleanup_runtime_outlives_manager_while_job_pending#settlement_cleanup_runtime_dies_with_manager#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-cleanup-runtime-lifetime.out 2>&1; then
  fail "checker accepted a cleanup runtime tied to manager lifetime"
fi
grep -q "setup cleanup runtime must outlive manager shutdown" /tmp/check-remoteapp-product-closure-cleanup-runtime-lifetime.out || \
  fail "expected setup cleanup runtime lifetime failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#failed_settlement_closes_admission_and_projects_typed_health#failed_settlement_keeps_admission_open#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-quarantine-health.out 2>&1; then
  fail "checker accepted settlement quarantine without fail-closed health coverage"
fi
grep -q "quarantine must close admission and expose typed health" /tmp/check-remoteapp-product-closure-quarantine-health.out || \
  fail "expected typed quarantine health failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#quarantine_projection_retries_after_queue_becomes_idle#quarantine_projection_stalls_after_queue_becomes_idle#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-quarantine-retry.out 2>&1; then
  fail "checker accepted an idle executor that cannot retry quarantine projection"
fi
grep -q "idle settlement executor must retry" /tmp/check-remoteapp-product-closure-quarantine-retry.out || \
  fail "expected quarantine projection retry failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#executor shutdown cannot drop quarantined transport ownership#executor shutdown may drop quarantined transport ownership#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-quarantine-lifetime.out 2>&1; then
  fail "checker accepted quarantine ownership loss after settlement queue shutdown"
fi
grep -q "settlement quarantine must outlive every manager submission handle" /tmp/check-remoteapp-product-closure-quarantine-lifetime.out || \
  fail "expected settlement quarantine lifetime failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#direct_endpoint_settlement_is_bounded_when_worker_does_not_exit#direct_endpoint_settlement_may_block#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-direct-settlement-deadline.out 2>&1; then
  fail "checker accepted direct WebRTC settlement without hung-worker regression coverage"
fi
grep -q "direct WebRTC settlement must prove a hung platform worker cannot hang shutdown" /tmp/check-remoteapp-product-closure-direct-settlement-deadline.out || \
  fail "expected bounded direct WebRTC settlement failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#pending_reservation_timeout_retains_completion_ownership#pending_reservation_timeout_drops_completion#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-pending-ownership.out 2>&1; then
  fail "checker accepted pending endpoint timeout without retained completion ownership"
fi
grep -q "pending endpoint timeout must retain completion ownership" /tmp/check-remoteapp-product-closure-pending-ownership.out || \
  fail "expected pending endpoint completion-ownership failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#settlement_executor_quarantines_panicking_job_without_dropping_owner#settlement_executor_drops_panicking_job#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-settlement-quarantine.out 2>&1; then
  fail "checker accepted a settlement executor without panic quarantine coverage"
fi
grep -q "settlement executor must prove a failed job is quarantined without owner loss" /tmp/check-remoteapp-product-closure-settlement-quarantine.out || \
  fail "expected settlement executor quarantine failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#admission_permit_linearizes_before_quarantine_projection#admission_snapshot_races_quarantine_projection#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-admission-linearization.out 2>&1; then
  fail "checker accepted session admission without quarantine linearization coverage"
fi
grep -q "quarantine waits for an already-admitted insert boundary" /tmp/check-remoteapp-product-closure-admission-linearization.out || \
  fail "expected settlement admission linearization failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#executor_unavailable_never_projects_quarantine_on_submitter#executor_unavailable_projects_quarantine_on_submitter#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-executor-fallback.out 2>&1; then
  fail "checker accepted synchronous executor-unavailable quarantine projection"
fi
grep -q "cannot re-enter session state on the submitter" /tmp/check-remoteapp-product-closure-executor-fallback.out || \
  fail "expected executor-unavailable projection-boundary failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#endpoint_cleanup_quarantine_emits_negative_completion_receipt#endpoint_cleanup_quarantine_strands_parent_closing#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-cleanup-negative-receipt.out 2>&1; then
  fail "checker accepted endpoint cleanup quarantine without a negative receipt"
fi
grep -q "parent receives an explicit negative receipt" /tmp/check-remoteapp-product-closure-cleanup-negative-receipt.out || \
  fail "expected endpoint cleanup negative-receipt failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#negative_receipt_does_not_discard_remaining_transport_ownership#negative_receipt_discards_remaining_transport_ownership#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-negative-receipt-ownership.out 2>&1; then
  fail "checker accepted negative settlement receipt ownership loss"
fi
grep -q "negative settlement receipts must not discard other reservation or worker ownership" /tmp/check-remoteapp-product-closure-negative-receipt-ownership.out || \
  fail "expected negative receipt ownership-retention failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#endpoint_cleanup_remains_visible_to_concurrent_terminal_settlement#endpoint_cleanup_becomes_invisible_before_terminal_settlement#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-setup-cleanup-terminal-race.out 2>&1; then
  fail "checker accepted setup cleanup that terminal settlement cannot observe"
fi
grep -q "setup cleanup remains manager-visible until its real close receipt" /tmp/check-remoteapp-product-closure-setup-cleanup-terminal-race.out || \
  fail "expected setup cleanup terminal-visibility failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#complete_with_endpoint_cleanup#complete_by_dropping_endpoint#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-setup-cleanup-owner.out 2>&1; then
  fail "checker accepted partially-created WebRTC peers without a cleanup owner"
fi
grep -q "partially-created WebRTC peers must transfer into a retained cleanup owner" /tmp/check-remoteapp-product-closure-setup-cleanup-owner.out || \
  fail "expected WebRTC setup cleanup-owner failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#endpoint_setup_phase_is_interrupted_by_terminal_admission_cancel#endpoint_setup_ignores_terminal_cancel#g' \
  "$SB/plugins/remote-desktop/src/transport/webrtc_endpoint.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-setup-cancel.out 2>&1; then
  fail "checker accepted WebRTC setup without terminal cancellation regression coverage"
fi
grep -q "setup must prove terminal cancellation interrupts a pending phase" /tmp/check-remoteapp-product-closure-setup-cancel.out || \
  fail "expected WebRTC setup cancellation failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/webrtc_endpoint.rs" "$SB/plugins/remote-desktop/src/transport/webrtc_endpoint.rs"

perl -0pi -e 's#deferred_settler_retains_ownership_and_finishes_after_initial_timeout#deferred_settler_drops_ownership_after_timeout#g' \
  "$SB/plugins/remote-desktop/src/session_lifecycle.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-deferred-settler.out 2>&1; then
  fail "checker accepted deferred settlement without timeout ownership regression coverage"
fi
grep -q "terminal lifecycle must prove a timeout retains ownership" /tmp/check-remoteapp-product-closure-deferred-settler.out || \
  fail "expected deferred settlement ownership failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/session_lifecycle.rs" "$SB/plugins/remote-desktop/src/session_lifecycle.rs"

perl -0pi -e 's#terminal_promotion_blocks_only_its_session_commit_boundary#terminal_promotion_blocks_global_session_store#g' \
  "$SB/plugins/remote-desktop/src/session_lifecycle.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-terminal-lock-boundary.out 2>&1; then
  fail "checker accepted terminal recovery I/O under the global session lock"
fi
grep -q "slow recovery I/O does not block unrelated session access" /tmp/check-remoteapp-product-closure-terminal-lock-boundary.out || \
  fail "expected terminal per-session commit-boundary failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/session_lifecycle.rs" "$SB/plugins/remote-desktop/src/session_lifecycle.rs"

perl -0pi -e 's#recovery\.promote\(staged\)#Ok::<_, anyhow::Error>(std::path::PathBuf::new())#g' \
  "$SB/plugins/remote-desktop/src/session_lifecycle.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-terminal-stage-promote.out 2>&1; then
  fail "checker accepted terminal publication without staged promotion"
fi
grep -q "atomically promote the CAS-matched staged candidate" /tmp/check-remoteapp-product-closure-terminal-stage-promote.out || \
  fail "expected terminal staged-promotion failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/session_lifecycle.rs" "$SB/plugins/remote-desktop/src/session_lifecycle.rs"

perl -0pi -e 's#stale_staged_terminal_never_replaces_newer_closing_revision#stale_staged_terminal_may_replace_newer_closing_revision#g' \
  "$SB/plugins/remote-desktop/src/session_lifecycle.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-terminal-stage-cas.out 2>&1; then
  fail "checker accepted terminal staging without stale-candidate regression coverage"
fi
grep -q "stale staged terminal never becomes recovery authority" /tmp/check-remoteapp-product-closure-terminal-stage-cas.out || \
  fail "expected stale terminal stage authority failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/session_lifecycle.rs" "$SB/plugins/remote-desktop/src/session_lifecycle.rs"

perl -0pi -e 's#commit_retry_delay#unbounded_retry_delay#g' \
  "$SB/plugins/remote-desktop/src/session_lifecycle.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-terminal-backoff.out 2>&1; then
  fail "checker accepted terminal persistence retry without backoff"
fi
grep -q "terminal persistence retries must use bounded exponential backoff" /tmp/check-remoteapp-product-closure-terminal-backoff.out || \
  fail "expected terminal persistence backoff failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/session_lifecycle.rs" "$SB/plugins/remote-desktop/src/session_lifecycle.rs"

perl -0pi -e 's#terminal_persistence_failure_retains_closing_until_retry_commits#terminal_persistence_failure_publishes_closed_without_retry#g' \
  "$SB/plugins/remote-desktop/src/session_lifecycle.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-terminal-persistence-retry.out 2>&1; then
  fail "checker accepted terminal publication without persistence-fault retry coverage"
fi
grep -q "retry durable publication after a persistence fault" /tmp/check-remoteapp-product-closure-terminal-persistence-retry.out || \
  fail "expected terminal persistence retry failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/session_lifecycle.rs" "$SB/plugins/remote-desktop/src/session_lifecycle.rs"

perl -0pi -e 's#\*session = terminal;#drop(terminal);#g' \
  "$SB/plugins/remote-desktop/src/session_lifecycle.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-terminal-publication-order.out 2>&1; then
  fail "checker accepted terminal persistence without publishing the durable candidate"
fi
grep -q "publish Closed only after durable persistence succeeds" /tmp/check-remoteapp-product-closure-terminal-publication-order.out || \
  fail "expected terminal publication-order failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/session_lifecycle.rs" "$SB/plugins/remote-desktop/src/session_lifecycle.rs"

perl -0pi -e 's#transports: Weak<RemoteDesktopTransportManager>#transports: Arc<RemoteDesktopTransportManager>#g' \
  "$SB/plugins/remote-desktop/src/transport/webrtc.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-handler-manager-cycle.out 2>&1; then
  fail "checker accepted a PeerConnection handler that strongly retains the transport manager"
fi
grep -q "callbacks must not strongly retain the transport manager" /tmp/check-remoteapp-product-closure-handler-manager-cycle.out || \
  fail "expected WebRTC handler-manager ownership-cycle failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/webrtc.rs" "$SB/plugins/remote-desktop/src/transport/webrtc.rs"

perl -0pi -e 's#peer_connection_handler_does_not_keep_transport_manager_alive#peer_connection_handler_keeps_transport_manager_alive#g' \
  "$SB/plugins/remote-desktop/src/transport/webrtc.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-handler-cycle-test.out 2>&1; then
  fail "checker accepted WebRTC callbacks without manager-cycle regression coverage"
fi
grep -q "callbacks must prove the manager-peer-handler ownership cycle is absent" /tmp/check-remoteapp-product-closure-handler-cycle-test.out || \
  fail "expected WebRTC handler ownership-cycle regression failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/webrtc.rs" "$SB/plugins/remote-desktop/src/transport/webrtc.rs"

perl -0pi -e 's#terminal_seal_cancels_and_settles_pending_endpoint_reservation#terminal_sweep_ignores_pending_endpoint#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-pending-endpoint-reservation.out 2>&1; then
  fail "checker accepted direct WebRTC setup without pending-reservation terminal regression coverage"
fi
grep -q "transport manager must prove terminal seal cancels and settles pending endpoint setup" /tmp/check-remoteapp-product-closure-pending-endpoint-reservation.out || \
  fail "expected pending direct WebRTC reservation regression failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#newer_endpoint_reservation_cancels_and_fences_older_generation#older_endpoint_reservation_may_replace_newer_generation#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-endpoint-generation-fence.out 2>&1; then
  fail "checker accepted direct WebRTC admission without out-of-order generation regression coverage"
fi
grep -q "transport manager must prove an older reservation cannot replace a newer endpoint generation" /tmp/check-remoteapp-product-closure-endpoint-generation-fence.out || \
  fail "expected direct WebRTC endpoint generation-fence failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#state\.sealed = true;#state.sealed = false;#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-terminal-admission-seal.out 2>&1; then
  fail "checker accepted a terminal transport sweep that does not seal endpoint admission"
fi
grep -q "terminal transport sweep must seal direct WebRTC endpoint admission" /tmp/check-remoteapp-product-closure-terminal-admission-seal.out || \
  fail "expected direct WebRTC terminal admission-seal failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/invoke_bidi.rs" "$SB/plugins/remote-desktop/src/invoke_bidi.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/handlers/attach.rs" "$SB/plugins/remote-desktop/src/handlers/attach.rs"

perl -0pi -e 's#async fn send_bidi_output_or_stop#async fn send_bidi_output_without_stop#g' \
  "$SB/plugins/remote-desktop/src/invoke_bidi.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-bidi-backpressure.out 2>&1; then
  fail "checker accepted diagnostic output without a stop-aware backpressure boundary"
fi
grep -q "diagnostic output must share one stop-aware bounded-queue publication boundary" /tmp/check-remoteapp-product-closure-bidi-backpressure.out || \
  fail "expected stop-aware diagnostic output backpressure failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/invoke_bidi.rs" "$SB/plugins/remote-desktop/src/invoke_bidi.rs"

perl -0pi -e 's#completion: Receiver<PreviewTaskGroupCompletion>#completion: ()#g' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-preview-completion.out 2>&1; then
  fail "checker accepted diagnostic preview ownership without a completion receipt"
fi
grep -q "diagnostic preview ownership must retain a worker-group completion receipt" /tmp/check-remoteapp-product-closure-preview-completion.out || \
  fail "expected diagnostic preview completion-ownership failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/manager.rs" "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's#const BIDI_TERMINAL_SEND_DEADLINE:#const UNBOUNDED_TERMINAL_SEND:#g' \
  "$SB/plugins/remote-desktop/src/transport/terminal.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-terminal-backpressure.out 2>&1; then
  fail "checker accepted diagnostic terminal publication without a bounded backpressure deadline"
fi
grep -q "terminal publication must have a bounded client-backpressure deadline" /tmp/check-remoteapp-product-closure-terminal-backpressure.out || \
  fail "expected bounded diagnostic terminal publication failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/transport/terminal.rs" "$SB/plugins/remote-desktop/src/transport/terminal.rs"

perl -0pi -e 's#bidi_wire_kind = "metadata_json_plus_binary"#bidi_wire_kind = "json_frames"#' \
  "$SB/plugins/remote-desktop/plugin.toml"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-attach-wire-manifest.out 2>&1; then
  fail "checker accepted RemoteApp attach manifest with JSON-only bidi framing"
fi
grep -q "RemoteApp attach manifest must declare metadata_json_plus_binary bidi media framing" /tmp/check-remoteapp-product-closure-attach-wire-manifest.out || \
  fail "expected RemoteApp attach manifest wire-kind failure"
cp "$REPO_ROOT/plugins/remote-desktop/plugin.toml" "$SB/plugins/remote-desktop/plugin.toml"

perl -0pi -e 's#PluginBidiWireKind::MetadataJsonPlusBinary#PluginBidiWireKind::JsonFrames#' \
  "$SB/plugins/remote-desktop/src/registration.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-attach-wire-registration.out 2>&1; then
  fail "checker accepted RemoteApp compiled attach spec with JSON-only bidi framing"
fi
grep -q "RemoteApp compiled attach spec must declare metadata/binary bidi framing" /tmp/check-remoteapp-product-closure-attach-wire-registration.out || \
  fail "expected RemoteApp attach registration wire-kind failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/registration.rs" "$SB/plugins/remote-desktop/src/registration.rs"

perl -0pi -e 's#bidi_wire_kind: ability\.bidi_wire_kind\(\)\.map\(Into::into\),#bidi_wire_kind: None,#' \
  "$SB/src/daemon/plugins/surface.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-surface-wire-projection.out 2>&1; then
  fail "checker accepted plugin surface without declared bidi wire-kind projection"
fi
grep -q "plugin surface must project declared bidi wire kind" /tmp/check-remoteapp-product-closure-surface-wire-projection.out || \
  fail "expected plugin surface wire-kind projection failure"
cp "$REPO_ROOT/src/daemon/plugins/surface.rs" "$SB/src/daemon/plugins/surface.rs"

perl -0pi -e 's#metadata_json_plus_binary#json_frames#' \
  "$SB/src/daemon/ability/builtins/real_invoke_tests.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-real-plugin-status-wire.out 2>&1; then
  fail "checker accepted real plugin.status test without RemoteApp metadata_json_plus_binary assertion"
fi
grep -q "real plugin.status test must assert metadata_json_plus_binary" /tmp/check-remoteapp-product-closure-real-plugin-status-wire.out || \
  fail "expected real plugin.status wire-kind assertion failure"
cp "$REPO_ROOT/src/daemon/ability/builtins/real_invoke_tests.rs" "$SB/src/daemon/ability/builtins/real_invoke_tests.rs"

perl -0pi -e 's/full RemoteApp product closure incomplete as of [0-9-]+/implemented; full acceptance verified 2026-08-24/' \
  "$SB/docs/design/remoteapp-targeted-session-spec.md"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-status.out 2>&1; then
  fail "checker accepted targeted-session SPEC that claims full product acceptance"
fi
grep -Eq "must not claim full product acceptance|must state that full RemoteApp product closure is incomplete" /tmp/check-remoteapp-product-closure-status.out || \
  fail "expected status misclaim failure"

cp "$REPO_ROOT/docs/design/remoteapp-targeted-session-spec.md" "$SB/docs/design/remoteapp-targeted-session-spec.md"
perl -0pi -e 's#Cross-device E2E smoke/regression exists beyond local provider boundary#Cross-device local-only smoke#' \
  "$SB/docs/design/remoteapp-product-readiness-audit-2026-08-22.md"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-cross-device.out 2>&1; then
  fail "checker accepted audit without cross-device product proof row"
fi
grep -q "cross-device proof" /tmp/check-remoteapp-product-closure-cross-device.out || \
  fail "expected cross-device audit failure"

cp "$REPO_ROOT/docs/design/remoteapp-product-readiness-audit-2026-08-22.md" "$SB/docs/design/remoteapp-product-readiness-audit-2026-08-22.md"
perl -0pi -e 's#does not prove real OS window/application capture#proves real OS window/application capture#g' \
  "$SB/tools/scripts/remoteapp-cross-device-product-smoke.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-cross-device-gate.out 2>&1; then
  fail "checker accepted cross-device smoke without product non-claims"
fi
grep -q "cross-device smoke must preserve product non-claims" /tmp/check-remoteapp-product-closure-cross-device-gate.out || \
  fail "expected cross-device smoke non-claim failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-device-product-smoke.sh" "$SB/tools/scripts/remoteapp-cross-device-product-smoke.sh"

perl -0pi -e 's#distinct_device_uras_observed#same_device_uras_observed#g' \
  "$SB/tools/scripts/remoteapp-cross-device-product-smoke.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-cross-device-distinct.out 2>&1; then
  fail "checker accepted cross-device smoke without distinct-device topology evidence"
fi
grep -q "cross-device smoke must report whether distinct device URAs were observed" /tmp/check-remoteapp-product-closure-cross-device-distinct.out || \
  fail "expected cross-device distinct-device topology failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-device-product-smoke.sh" "$SB/tools/scripts/remoteapp-cross-device-product-smoke.sh"

perl -0pi -e 's#distinct device URAs were not observed#distinct device URAs are optional#g' \
  "$SB/tools/scripts/remoteapp-cross-device-product-smoke.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-cross-device-hard-gate.out 2>&1; then
  fail "checker accepted cross-device smoke without local-only failure gate"
fi
grep -q "cross-device smoke must fail when distinct device URAs are not observed" /tmp/check-remoteapp-product-closure-cross-device-hard-gate.out || \
  fail "expected cross-device local-only hard-gate failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-device-product-smoke.sh" "$SB/tools/scripts/remoteapp-cross-device-product-smoke.sh"

perl -0pi -e 's#local_provider_boundary_only=true#local_provider_boundary_only=false#g' \
  "$SB/tools/scripts/remoteapp-cross-device-product-smoke.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-cross-device-local-only.out 2>&1; then
  fail "checker accepted cross-device smoke without local-provider-only failure reason"
fi
grep -q "cross-device smoke must fail local-provider-only passed runs" /tmp/check-remoteapp-product-closure-cross-device-local-only.out || \
  fail "expected cross-device local-provider-only failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-device-product-smoke.sh" "$SB/tools/scripts/remoteapp-cross-device-product-smoke.sh"

perl -0pi -e 's#product_complete_claim.*False#product_complete_claim\": True#g' \
  "$SB/tools/scripts/remoteapp-cross-device-product-smoke.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-cross-device-complete-claim.out 2>&1; then
  fail "checker accepted cross-device smoke with product completion claim"
fi
grep -q "cross-device smoke must reject product completion claims" /tmp/check-remoteapp-product-closure-cross-device-complete-claim.out || \
  fail "expected cross-device product completion claim failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-device-product-smoke.sh" "$SB/tools/scripts/remoteapp-cross-device-product-smoke.sh"

perl -0pi -e 's#and campaign_verified#and True#g' \
  "$SB/tools/scripts/remoteapp-product-completion-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-signed-campaign-claim.out 2>&1; then
  fail "checker accepted product-completion claims without signed campaign verification"
fi
grep -q "eligibility must be gated by verified signed campaign evidence" /tmp/check-remoteapp-product-closure-signed-campaign-claim.out || \
  fail "expected signed campaign claim-gate failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-product-completion-e2e.sh" "$SB/tools/scripts/remoteapp-product-completion-e2e.sh"

perl -0pi -e 's#COMPLETION_ROLE = "product_completion_authority"#COMPLETION_ROLE = "campaign_authority"#g' \
  "$SB/tools/scripts/remoteapp-product-finalize.py"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-completion-authority-role.out 2>&1; then
  fail "checker accepted final claims under the campaign authority role"
fi
grep -q "independent product-completion authority" /tmp/check-remoteapp-product-closure-completion-authority-role.out || \
  fail "expected independent completion-authority role failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-product-finalize.py" "$SB/tools/scripts/remoteapp-product-finalize.py"

perl -0pi -e 's#"product_complete_claim": True#"product_complete_claim": False#g' \
  "$SB/tools/scripts/remoteapp-product-finalize.py"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-final-claim.out 2>&1; then
  fail "checker accepted a finalizer that cannot emit the authorized final claim"
fi
grep -q "only the signed completion finalizer may mint" /tmp/check-remoteapp-product-closure-final-claim.out || \
  fail "expected signed final-claim boundary failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-product-finalize.py" "$SB/tools/scripts/remoteapp-product-finalize.py"

perl -0pi -e 's#EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON#EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CAPTURE_OPTIONAL_JSON#g' \
  "$SB/tools/scripts/remoteapp-product-completion-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-completion-capture-env.out 2>&1; then
  fail "checker accepted product-completion gate without cross-platform capture report requirement"
fi
grep -q "product-completion gate must require cross-platform capture evidence" /tmp/check-remoteapp-product-closure-completion-capture-env.out || \
  fail "expected product-completion cross-platform capture env failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-product-completion-e2e.sh" "$SB/tools/scripts/remoteapp-product-completion-e2e.sh"

perl -0pi -e 's#requires_cross_platform_capture_scenarios#requires_capture_platforms_only#g' \
  "$SB/tools/scripts/remoteapp-product-completion-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-completion-capture-scenarios.out 2>&1; then
  fail "checker accepted product-completion gate without cross-platform capture scenarios"
fi
grep -q "product-completion gate must require cross-platform capture scenario summaries" /tmp/check-remoteapp-product-closure-completion-capture-scenarios.out || \
  fail "expected product-completion cross-platform capture scenarios failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-product-completion-e2e.sh" "$SB/tools/scripts/remoteapp-product-completion-e2e.sh"

perl -0pi -e 's#requires_frontend_flow_summary#requires_frontend_steps_only#g' \
  "$SB/tools/scripts/remoteapp-product-completion-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-completion-frontend-summary.out 2>&1; then
  fail "checker accepted product-completion gate without frontend flow summaries"
fi
grep -q "product-completion gate must require frontend product-flow summaries" /tmp/check-remoteapp-product-closure-completion-frontend-summary.out || \
  fail "expected product-completion frontend flow summary failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-product-completion-e2e.sh" "$SB/tools/scripts/remoteapp-product-completion-e2e.sh"

perl -0pi -e 's#requires_input_injection_scenarios#requires_input_platform_status_only#g' \
  "$SB/tools/scripts/remoteapp-product-completion-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-completion-input-scenarios.out 2>&1; then
  fail "checker accepted product-completion gate without input injection summaries"
fi
grep -q "product-completion gate must require input injection summaries" /tmp/check-remoteapp-product-closure-completion-input-scenarios.out || \
  fail "expected product-completion input injection summaries failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-product-completion-e2e.sh" "$SB/tools/scripts/remoteapp-product-completion-e2e.sh"

perl -0pi -e 's#requires_cross_device_remoteapp_scenarios#requires_cross_device_targets_only#g' \
  "$SB/tools/scripts/remoteapp-product-completion-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-completion-cross-device-remoteapp-scenarios.out 2>&1; then
  fail "checker accepted product-completion gate without cross-device RemoteApp summaries"
fi
grep -q "product-completion gate must require cross-device RemoteApp target summaries" /tmp/check-remoteapp-product-closure-completion-cross-device-remoteapp-scenarios.out || \
  fail "expected product-completion cross-device RemoteApp summaries failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-product-completion-e2e.sh" "$SB/tools/scripts/remoteapp-product-completion-e2e.sh"

perl -0pi -e 's#topology.local_provider_boundary_only is not false#topology.local_provider_boundary_only is optional#g' \
  "$SB/tools/scripts/remoteapp-product-completion-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-completion-local-only.out 2>&1; then
  fail "checker accepted product-completion gate without local-provider-only rejection"
fi
grep -q "product-completion gate must reject local-provider-only cross-device reports" /tmp/check-remoteapp-product-closure-completion-local-only.out || \
  fail "expected product-completion local-provider-only rejection failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-product-completion-e2e.sh" "$SB/tools/scripts/remoteapp-product-completion-e2e.sh"

perl -0pi -e 's#requires_lifecycle_summary#requires_lifecycle_detail#g' \
  "$SB/tools/scripts/remoteapp-product-completion-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-completion-lifecycle-summary.out 2>&1; then
  fail "checker accepted product-completion gate without lifecycle summary requirement"
fi
grep -q "product-completion gate must require lifecycle summary evidence" /tmp/check-remoteapp-product-closure-completion-lifecycle-summary.out || \
  fail "expected product-completion lifecycle summary failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-product-completion-e2e.sh" "$SB/tools/scripts/remoteapp-product-completion-e2e.sh"

perl -0pi -e 's#frontend_flow_summary#frontend_step_summary#g' \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-frontend-flow-summary.out 2>&1; then
  fail "checker accepted frontend product-flow verifier without journey summaries"
fi
grep -q "frontend product-flow verifier must emit product journey summaries" /tmp/check-remoteapp-product-closure-frontend-flow-summary.out || \
  fail "expected frontend product-flow summary failure"
cp "$REPO_ROOT/tools/scripts/frontend-remoteapp-product-flow-e2e.sh" "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"

perl -0pi -e 's#real_cross_platform_capture_matrix#source_only_capture_matrix#g' \
  "$SB/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-capture-proof-mode.out 2>&1; then
  fail "checker accepted cross-platform capture verifier without real capture proof mode"
fi
grep -q "cross-platform capture verifier must require real capture matrix proof mode" /tmp/check-remoteapp-product-closure-capture-proof-mode.out || \
  fail "expected cross-platform capture proof-mode failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-platform-capture-e2e.sh" "$SB/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"

perl -0pi -e 's#macos must pass display/window/application capture#macos may report unsupported capture#g' \
  "$SB/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-capture-macos.out 2>&1; then
  fail "checker accepted cross-platform capture verifier without macOS pass requirement"
fi
grep -q "cross-platform capture verifier must require macOS live capture pass" /tmp/check-remoteapp-product-closure-capture-macos.out || \
  fail "expected macOS capture pass requirement failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-platform-capture-e2e.sh" "$SB/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"

perl -0pi -e 's#first_display_capture_started#display_capture_started#g' \
  "$SB/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-capture-fallback.out 2>&1; then
  fail "checker accepted cross-platform capture verifier without first-display fallback evidence"
fi
grep -q "cross-platform capture verifier must inspect first-display fallback evidence" /tmp/check-remoteapp-product-closure-capture-fallback.out || \
  fail "expected first-display fallback evidence failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-platform-capture-e2e.sh" "$SB/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"

perl -0pi -e 's#selected_sentinel_rendered#selected_sentinel_visible#g' \
  "$SB/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-capture-selected-sentinel.out 2>&1; then
  fail "checker accepted cross-platform capture verifier without selected sentinel evidence"
fi
grep -q "cross-platform capture verifier must require selected sentinel render evidence" /tmp/check-remoteapp-product-closure-capture-selected-sentinel.out || \
  fail "expected selected sentinel render evidence failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-platform-capture-e2e.sh" "$SB/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"

perl -0pi -e 's#target_identity#target_descriptor#g' \
  "$SB/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-capture-target-identity.out 2>&1; then
  fail "checker accepted cross-platform capture verifier without selected target identity evidence"
fi
grep -q "cross-platform capture verifier must require selected target identity evidence" /tmp/check-remoteapp-product-closure-capture-target-identity.out || \
  fail "expected capture target identity failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-platform-capture-e2e.sh" "$SB/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"

perl -0pi -e 's#rendered_frame_probe frame_source_id must match target_identity#rendered_frame_probe frame_source_id may differ#g' \
  "$SB/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-capture-frame-source.out 2>&1; then
  fail "checker accepted cross-platform capture verifier without frame-source binding"
fi
grep -q "cross-platform capture verifier must bind rendered frame source to target identity" /tmp/check-remoteapp-product-closure-capture-frame-source.out || \
  fail "expected capture frame-source binding failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-platform-capture-e2e.sh" "$SB/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"

perl -0pi -e 's#selected_sentinel_hash#selected_sentinel_checksum#g' \
  "$SB/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-capture-sentinel-hash.out 2>&1; then
  fail "checker accepted cross-platform capture verifier without sentinel hash evidence"
fi
grep -q "cross-platform capture verifier must require selected sentinel hash evidence" /tmp/check-remoteapp-product-closure-capture-sentinel-hash.out || \
  fail "expected capture sentinel hash failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-platform-capture-e2e.sh" "$SB/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"

perl -0pi -e 's#unrelated_sentinel_rendered#unrelated_sentinel_visible#g' \
  "$SB/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-capture-unrelated-sentinel.out 2>&1; then
  fail "checker accepted cross-platform capture verifier without unrelated sentinel leakage rejection"
fi
grep -q "cross-platform capture verifier must reject unrelated sentinel leakage" /tmp/check-remoteapp-product-closure-capture-unrelated-sentinel.out || \
  fail "expected unrelated sentinel leakage rejection failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-platform-capture-e2e.sh" "$SB/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"

perl -0pi -e 's#real_input_injection_matrix#policy_only_input_matrix#g' \
  "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-input-proof-mode.out 2>&1; then
  fail "checker accepted input injection verifier without real input proof mode"
fi
grep -q "input injection verifier must require real input injection proof mode" /tmp/check-remoteapp-product-closure-input-proof-mode.out || \
  fail "expected input injection proof-mode failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-input-injection-e2e.sh" "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"

perl -0pi -e 's#macos must pass pointer/keyboard input injection#macos may skip input injection#g' \
  "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-input-macos.out 2>&1; then
  fail "checker accepted input injection verifier without macOS pass requirement"
fi
grep -q "input injection verifier must require macOS live input pass" /tmp/check-remoteapp-product-closure-input-macos.out || \
  fail "expected macOS input pass requirement failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-input-injection-e2e.sh" "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"

perl -0pi -e 's#latency_ms must be within threshold#latency_ms may exceed threshold#g' \
  "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-input-latency.out 2>&1; then
  fail "checker accepted input injection verifier without latency bound"
fi
grep -q "input injection verifier must reject high latency" /tmp/check-remoteapp-product-closure-input-latency.out || \
  fail "expected input injection latency-bound failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-input-injection-e2e.sh" "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"

perl -0pi -e 's#input_results client_sequence must be strictly increasing#input_results client_sequence may repeat#g' \
  "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-input-sequence-order.out 2>&1; then
  fail "checker accepted input injection verifier without monotonic applied sequence validation"
fi
grep -q "input injection verifier must reject non-monotonic applied input sequences" /tmp/check-remoteapp-product-closure-input-sequence-order.out || \
  fail "expected input injection sequence-order failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-input-injection-e2e.sh" "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"

perl -0pi -e 's#stale_client_sequence rejection must be observed#stale_client_sequence rejection is optional#g' \
  "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-input-stale-rejection.out 2>&1; then
  fail "checker accepted input injection verifier without stale sequence rejection evidence"
fi
grep -q "input injection verifier must require stale sequence rejection evidence" /tmp/check-remoteapp-product-closure-input-stale-rejection.out || \
  fail "expected input injection stale sequence rejection failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-input-injection-e2e.sh" "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"

perl -0pi -e 's#target_focus_epoch must be positive#target_focus_epoch may be absent#g' \
  "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-input-focus-epoch.out 2>&1; then
  fail "checker accepted input injection verifier without focus epoch evidence"
fi
grep -q "input injection verifier must require focused target epoch evidence" /tmp/check-remoteapp-product-closure-input-focus-epoch.out || \
  fail "expected input injection focus-epoch failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-input-injection-e2e.sh" "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"

perl -0pi -e 's#input_event_id must be recorded#input_event_id may be absent#g' \
  "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-input-event-id.out 2>&1; then
  fail "checker accepted input injection verifier without stable input event identity"
fi
grep -q "input injection verifier must require stable input event identity" /tmp/check-remoteapp-product-closure-input-event-id.out || \
  fail "expected input injection event-id failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-input-injection-e2e.sh" "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"

perl -0pi -e 's#os_effect_probe_source#telemetry_probe_source#g' \
  "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-input-os-effect-source.out 2>&1; then
  fail "checker accepted input injection verifier without platform OS-effect observer evidence"
fi
grep -q "input injection verifier must require platform OS-effect observer evidence" /tmp/check-remoteapp-product-closure-input-os-effect-source.out || \
  fail "expected input injection OS-effect observer failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-input-injection-e2e.sh" "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"

perl -0pi -e 's#os_effect observer must be independent from injector#os_effect observer may be injector#g' \
  "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-input-observer-independence.out 2>&1; then
  fail "checker accepted input injection verifier without independent OS-effect observer evidence"
fi
grep -q "input injection verifier must require independent OS-effect observer evidence" /tmp/check-remoteapp-product-closure-input-observer-independence.out || \
  fail "expected input injection observer-independence failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-input-injection-e2e.sh" "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"

perl -0pi -e 's#os_effect input_event_id must bind input_event_id#os_effect input_event_id may differ#g' \
  "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-input-effect-event-id.out 2>&1; then
  fail "checker accepted input injection verifier without OS-effect input event binding"
fi
grep -q "input injection verifier must bind OS effect to the applied input event" /tmp/check-remoteapp-product-closure-input-effect-event-id.out || \
  fail "expected input injection OS-effect event-id binding failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-input-injection-e2e.sh" "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"

perl -0pi -e 's#os_effect observed_at_ms must be after host_applied_at_ms#os_effect observed_at_ms may precede host_applied_at_ms#g' \
  "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-input-os-effect-order.out 2>&1; then
  fail "checker accepted input injection verifier without post-application OS-effect timing"
fi
grep -q "input injection verifier must require OS effect after host application" /tmp/check-remoteapp-product-closure-input-os-effect-order.out || \
  fail "expected input injection OS-effect timing failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-input-injection-e2e.sh" "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"

perl -0pi -e 's#os_effect target_focus_epoch must match platform scenario#os_effect focus epoch may differ#g' \
  "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-input-effect-focus-epoch.out 2>&1; then
  fail "checker accepted input injection verifier without OS-effect focus epoch binding"
fi
grep -q "input injection verifier must bind OS effect to target focus epoch" /tmp/check-remoteapp-product-closure-input-effect-focus-epoch.out || \
  fail "expected input injection OS-effect focus epoch failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-input-injection-e2e.sh" "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"

perl -0pi -e 's#pointer OS effect must be observed within tolerance#pointer OS effect tolerance is optional#g' \
  "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-input-pointer-effect.out 2>&1; then
  fail "checker accepted input injection verifier without bounded pointer OS-effect evidence"
fi
grep -q "input injection verifier must require bounded pointer OS effect evidence" /tmp/check-remoteapp-product-closure-input-pointer-effect.out || \
  fail "expected input injection bounded pointer OS-effect failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-input-injection-e2e.sh" "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"

perl -0pi -e 's#keyboard OS effect must bind focused Resource URA#keyboard OS effect focus binding is optional#g' \
  "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-input-keyboard-effect.out 2>&1; then
  fail "checker accepted input injection verifier without keyboard focus/resource binding"
fi
grep -q "input injection verifier must require keyboard focus/resource binding" /tmp/check-remoteapp-product-closure-input-keyboard-effect.out || \
  fail "expected input injection keyboard focus/resource failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-input-injection-e2e.sh" "$SB/tools/scripts/remoteapp-input-injection-e2e.sh"

perl -0pi -e 's#remoteapp_summary#remoteapp_status_summary#g' \
  "$SB/tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-cross-device-remoteapp-summary.out 2>&1; then
  fail "checker accepted cross-device RemoteApp verifier without aggregate summaries"
fi
grep -q "cross-device RemoteApp verifier must emit product aggregate summaries" /tmp/check-remoteapp-product-closure-cross-device-remoteapp-summary.out || \
  fail "expected cross-device RemoteApp summary failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh" "$SB/tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh"

perl -0pi -e 's#real_media_adaptation_matrix#source_only_media_matrix#g' \
  "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-media-proof-mode.out 2>&1; then
  fail "checker accepted media adaptation verifier without real media proof mode"
fi
grep -q "media adaptation verifier must require real media adaptation proof mode" /tmp/check-remoteapp-product-closure-media-proof-mode.out || \
  fail "expected media adaptation proof-mode failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"

perl -0pi -e 's#\{"webrtc", "native_webrtc"\}#{"webrtc", "native_webrtc", "raw_stream_v8"}#g' \
  "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-media-transport.out 2>&1; then
  fail "checker accepted generic raw stream ABI as RemoteApp media transport evidence"
fi
grep -q "media adaptation verifier must reject generic raw-stream ABI as RemoteApp media proof" /tmp/check-remoteapp-product-closure-media-transport.out || \
  fail "expected RemoteApp WebRTC transport boundary failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"

perl -0pi -e 's#bitrate_downshift#bitrate_hint#g' \
  "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-media-bitrate.out 2>&1; then
  fail "checker accepted media adaptation verifier without bitrate downshift evidence"
fi
grep -q "media adaptation verifier must require bitrate downshift evidence" /tmp/check-remoteapp-product-closure-media-bitrate.out || \
  fail "expected media adaptation bitrate-downshift failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"

perl -0pi -e 's#degraded_network target_bitrate_kbps must be lower than baseline#degraded_network target bitrate may match baseline#g' \
  "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-media-target-delta.out 2>&1; then
  fail "checker accepted media adaptation verifier without degraded target bitrate delta"
fi
grep -q "media adaptation verifier must require degraded target bitrate downshift" /tmp/check-remoteapp-product-closure-media-target-delta.out || \
  fail "expected media adaptation degraded target bitrate delta failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"

perl -0pi -e 's#media_pipeline_id must match across media scenarios#media_pipeline_id may differ across media scenarios#g' \
  "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-media-pipeline-match.out 2>&1; then
  fail "checker accepted media adaptation verifier without media pipeline comparability"
fi
grep -q "media adaptation verifier must compare one media pipeline across scenarios" /tmp/check-remoteapp-product-closure-media-pipeline-match.out || \
  fail "expected media adaptation pipeline comparability failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"

perl -0pi -e 's#scenario_started_at_ms must be recorded#scenario start is optional#g' \
  "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-media-scenario-start.out 2>&1; then
  fail "checker accepted media adaptation verifier without scenario start timestamp evidence"
fi
grep -q "media adaptation verifier must require scenario start timestamp evidence" /tmp/check-remoteapp-product-closure-media-scenario-start.out || \
  fail "expected media adaptation scenario start timestamp failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"

perl -0pi -e 's#impairment_applied_at_ms must be after scenario_started_at_ms#impairment timing is optional#g' \
  "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-media-impairment-time.out 2>&1; then
  fail "checker accepted media adaptation verifier without impairment timing evidence"
fi
grep -q "media adaptation verifier must require impairment timing evidence" /tmp/check-remoteapp-product-closure-media-impairment-time.out || \
  fail "expected media adaptation impairment timing failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"

perl -0pi -e 's#media_pipeline_id must bind media_pipeline_id#media event pipeline binding is optional#g' \
  "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-media-event-pipeline.out 2>&1; then
  fail "checker accepted media adaptation verifier without event media-pipeline binding"
fi
grep -q "media adaptation verifier must bind adaptation events to the media pipeline" /tmp/check-remoteapp-product-closure-media-event-pipeline.out || \
  fail "expected media adaptation event pipeline binding failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"

perl -0pi -e 's#at_ms must be after impairment_applied_at_ms#adaptation event may precede impairment#g' \
  "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-media-event-order.out 2>&1; then
  fail "checker accepted media adaptation verifier without event-after-impairment evidence"
fi
grep -q "media adaptation verifier must require adaptation events after impairment" /tmp/check-remoteapp-product-closure-media-event-order.out || \
  fail "expected media adaptation event-after-impairment failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"

perl -0pi -e 's#frames_rendered_after_adaptation_at_ms must be after adaptation events#rendered-after-adaptation timing is optional#g' \
  "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-media-render-after-event.out 2>&1; then
  fail "checker accepted media adaptation verifier without rendered-after-adaptation timing evidence"
fi
grep -q "media adaptation verifier must require rendered media after adaptation events" /tmp/check-remoteapp-product-closure-media-render-after-event.out || \
  fail "expected media adaptation rendered-after-adaptation timing failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"

perl -0pi -e 's#render_probe evidence must be present#render probe evidence may be absent#g' \
  "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-media-render-probe.out 2>&1; then
  fail "checker accepted media adaptation verifier without decoded render probe evidence"
fi
grep -q "media adaptation verifier must require decoded render probe evidence" /tmp/check-remoteapp-product-closure-media-render-probe.out || \
  fail "expected media adaptation render-probe failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"

perl -0pi -e 's#render_probe media_pipeline_id must bind media_pipeline_id#render probe pipeline binding is optional#g' \
  "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-media-render-pipeline.out 2>&1; then
  fail "checker accepted media adaptation verifier without render-probe pipeline binding"
fi
grep -q "media adaptation verifier must bind render probe to media pipeline" /tmp/check-remoteapp-product-closure-media-render-pipeline.out || \
  fail "expected media adaptation render-probe pipeline failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"

perl -0pi -e 's#render_probe audio_payload_hash must be recorded#render_probe audio_payload_hash may be absent#g' \
  "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-media-render-audio-hash.out 2>&1; then
  fail "checker accepted media adaptation verifier without audio payload fingerprint evidence"
fi
grep -q "media adaptation verifier must require audio payload fingerprint evidence" /tmp/check-remoteapp-product-closure-media-render-audio-hash.out || \
  fail "expected media adaptation audio payload fingerprint failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"

perl -0pi -e 's#render_probe observed_at_ms must be after adaptation events#render_probe observation ordering is optional#g' \
  "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-media-render-order.out 2>&1; then
  fail "checker accepted media adaptation verifier without render-probe ordering"
fi
grep -q "media adaptation verifier must order render probe after adaptation events" /tmp/check-remoteapp-product-closure-media-render-order.out || \
  fail "expected media adaptation render-probe ordering failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"

perl -0pi -e 's#selected_resource_ura must match across media scenarios#selected_resource_ura may differ across media scenarios#g' \
  "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-media-resource-match.out 2>&1; then
  fail "checker accepted media adaptation verifier without selected resource comparability"
fi
grep -q "media adaptation verifier must compare the same selected resource across scenarios" /tmp/check-remoteapp-product-closure-media-resource-match.out || \
  fail "expected media adaptation selected resource comparability failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"

perl -0pi -e 's#audio.status must be passed#audio.status may be unsupported#g' \
  "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-media-audio.out 2>&1; then
  fail "checker accepted media adaptation verifier without host audio pass requirement"
fi
grep -q "media adaptation verifier must require live host audio evidence" /tmp/check-remoteapp-product-closure-media-audio.out || \
  fail "expected media adaptation host-audio failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"

perl -0pi -e 's#queue.observed_max_depth must not exceed max_depth#queue may grow beyond max_depth#g' \
  "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-media-queue.out 2>&1; then
  fail "checker accepted media adaptation verifier without bounded queue evidence"
fi
grep -q "media adaptation verifier must reject unbounded queue evidence" /tmp/check-remoteapp-product-closure-media-queue.out || \
  fail "expected media adaptation bounded-queue failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh" "$SB/tools/scripts/remoteapp-media-adaptation-e2e.sh"

perl -0pi -e 's#real_multi_window_tracking_matrix#source_only_tracking_matrix#g' \
  "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-tracking-proof-mode.out 2>&1; then
  fail "checker accepted multi-window tracking verifier without real tracking proof mode"
fi
grep -q "multi-window tracking verifier must require real tracking proof mode" /tmp/check-remoteapp-product-closure-tracking-proof-mode.out || \
  fail "expected multi-window tracking proof-mode failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-multi-window-tracking-e2e.sh" "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"

perl -0pi -e 's#frames_interleaved must be false#frames_interleaved may be true#g' \
  "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-tracking-interleaved.out 2>&1; then
  fail "checker accepted multi-window tracking verifier without stream isolation"
fi
grep -q "multi-window tracking verifier must reject interleaved streams" /tmp/check-remoteapp-product-closure-tracking-interleaved.out || \
  fail "expected multi-window tracking stream-isolation failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-multi-window-tracking-e2e.sh" "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"

perl -0pi -e 's#selected_sentinel_rendered#selected_sentinel_visible#g' \
  "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-tracking-selected-sentinel.out 2>&1; then
  fail "checker accepted multi-window tracking verifier without selected sentinel rendering"
fi
grep -q "multi-window tracking verifier must require selected target sentinel rendering" /tmp/check-remoteapp-product-closure-tracking-selected-sentinel.out || \
  fail "expected multi-window tracking selected-sentinel rendering failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-multi-window-tracking-e2e.sh" "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"

perl -0pi -e 's#rendered_frame_probe must be present#rendered_frame_probe may be absent#g' \
  "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-tracking-frame-probe.out 2>&1; then
  fail "checker accepted multi-window tracking verifier without decoded per-stream frame probe"
fi
grep -q "multi-window tracking verifier must require decoded per-stream frame probe evidence" /tmp/check-remoteapp-product-closure-tracking-frame-probe.out || \
  fail "expected multi-window tracking frame-probe failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-multi-window-tracking-e2e.sh" "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"

perl -0pi -e 's#rendered_frame_probe frame_source_id must bind stream frame source#rendered_frame_probe frame_source_id may differ#g' \
  "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-tracking-frame-source.out 2>&1; then
  fail "checker accepted multi-window tracking verifier without frame-probe source binding"
fi
grep -q "multi-window tracking verifier must bind frame probe to stream frame source" /tmp/check-remoteapp-product-closure-tracking-frame-source.out || \
  fail "expected multi-window tracking frame-source binding failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-multi-window-tracking-e2e.sh" "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"

perl -0pi -e 's#rendered_frame_probe selected_sentinel_hash must be recorded#rendered_frame_probe selected_sentinel_hash may be absent#g' \
  "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-tracking-frame-sentinel-hash.out 2>&1; then
  fail "checker accepted multi-window tracking verifier without per-stream sentinel hash evidence"
fi
grep -q "multi-window tracking verifier must require per-stream sentinel hash evidence" /tmp/check-remoteapp-product-closure-tracking-frame-sentinel-hash.out || \
  fail "expected multi-window tracking sentinel hash failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-multi-window-tracking-e2e.sh" "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"

perl -0pi -e 's#rendered_frame_probe foreign_sentinel_rendered must be false#rendered_frame_probe foreign_sentinel_rendered may be true#g' \
  "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-tracking-frame-leakage.out 2>&1; then
  fail "checker accepted multi-window tracking verifier without decoded-probe foreign sentinel rejection"
fi
grep -q "multi-window tracking verifier must reject foreign sentinel leakage in decoded stream probe" /tmp/check-remoteapp-product-closure-tracking-frame-leakage.out || \
  fail "expected multi-window tracking decoded-probe leakage failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-multi-window-tracking-e2e.sh" "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"

perl -0pi -e 's#uncommitted_same_app_sentinel_rendered#uncommitted_same_app_sentinel_visible#g' \
  "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-tracking-uncommitted-sentinel.out 2>&1; then
  fail "checker accepted multi-window tracking verifier without same-app leakage rejection"
fi
grep -q "multi-window tracking verifier must reject uncommitted same-app window sentinel leakage" /tmp/check-remoteapp-product-closure-tracking-uncommitted-sentinel.out || \
  fail "expected multi-window tracking uncommitted-sentinel leakage failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-multi-window-tracking-e2e.sh" "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"

perl -0pi -e 's#PENDING_MEDIA_REBIND#PENDING_SOURCE_REFRESH#g' \
  "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-tracking-rebind.out 2>&1; then
  fail "checker accepted multi-window tracking verifier without pending media rebind evidence"
fi
grep -q "multi-window tracking verifier must inspect pending media rebind events" /tmp/check-remoteapp-product-closure-tracking-rebind.out || \
  fail "expected multi-window tracking rebind failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-multi-window-tracking-e2e.sh" "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"

perl -0pi -e 's#unsupported multi-display app must not start capture session#unsupported multi-display app may start fallback capture#g' \
  "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-tracking-unsupported.out 2>&1; then
  fail "checker accepted multi-window tracking verifier without unsupported capture-start rejection"
fi
grep -q "multi-window tracking verifier must reject unsupported capture start" /tmp/check-remoteapp-product-closure-tracking-unsupported.out || \
  fail "expected multi-window tracking unsupported capture failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-multi-window-tracking-e2e.sh" "$SB/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"

perl -0pi -e 's#real_network_fallback_matrix#route_model_source_check#g' \
  "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-network-proof-mode.out 2>&1; then
  fail "checker accepted network fallback verifier without real network fallback proof mode"
fi
grep -q "network fallback verifier must require real network fallback proof mode" /tmp/check-remoteapp-product-closure-network-proof-mode.out || \
  fail "expected network fallback proof-mode failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh" "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"

perl -0pi -e 's#selected_route_class#selected_route_label#g' \
  "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-network-route-class.out 2>&1; then
  fail "checker accepted network fallback verifier without selected route-class evidence"
fi
grep -q "network fallback verifier must inspect selected route-class evidence" /tmp/check-remoteapp-product-closure-network-route-class.out || \
  fail "expected network fallback selected route-class failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh" "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"

perl -0pi -e 's#route_constraints_applied#route_constraints_described#g' \
  "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-network-constraints.out 2>&1; then
  fail "checker accepted network fallback verifier without applied route constraints"
fi
grep -q "network fallback verifier must require route constraints to be applied" /tmp/check-remoteapp-product-closure-network-constraints.out || \
  fail "expected network fallback route-constraints failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh" "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"

perl -0pi -e 's#rendered_after_selected_pair#rendered_after_session_start#g' \
  "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-network-render-order.out 2>&1; then
  fail "checker accepted network fallback verifier without render-after-selected-pair evidence"
fi
grep -q "network fallback verifier must require rendered media after selected pair" /tmp/check-remoteapp-product-closure-network-render-order.out || \
  fail "expected network fallback render-order failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh" "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"

perl -0pi -e 's#selected_candidate_pair.nominated must be true#selected_candidate_pair.nominated may be false#g' \
  "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-network-nominated-pair.out 2>&1; then
  fail "checker accepted network fallback verifier without nominated ICE pair evidence"
fi
grep -q "network fallback verifier must require nominated ICE pair evidence" /tmp/check-remoteapp-product-closure-network-nominated-pair.out || \
  fail "expected network fallback nominated ICE pair failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh" "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"

perl -0pi -e 's#selected_candidate_pair.state must be succeeded#selected_candidate_pair.state may be in-progress#g' \
  "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-network-succeeded-pair.out 2>&1; then
  fail "checker accepted network fallback verifier without succeeded ICE pair evidence"
fi
grep -q "network fallback verifier must require succeeded ICE pair evidence" /tmp/check-remoteapp-product-closure-network-succeeded-pair.out || \
  fail "expected network fallback succeeded ICE pair failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh" "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"

perl -0pi -e 's#webrtc session_id must bind session_id#webrtc session_id may be external#g' \
  "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-network-webrtc-session.out 2>&1; then
  fail "checker accepted network fallback verifier without WebRTC session binding"
fi
grep -q "network fallback verifier must bind WebRTC evidence to RemoteApp session" /tmp/check-remoteapp-product-closure-network-webrtc-session.out || \
  fail "expected network fallback WebRTC session binding failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh" "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"

perl -0pi -e 's#selected_candidate_pair.candidate_pair_id must be recorded#selected_candidate_pair.candidate_pair_label may be recorded#g' \
  "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-network-candidate-pair-id.out 2>&1; then
  fail "checker accepted network fallback verifier without selected candidate-pair id evidence"
fi
grep -q "network fallback verifier must require selected candidate-pair id evidence" /tmp/check-remoteapp-product-closure-network-candidate-pair-id.out || \
  fail "expected network fallback candidate-pair id failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh" "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"

perl -0pi -e 's#media candidate_pair_id must match selected_candidate_pair#media candidate_pair_id may differ from selected_candidate_pair#g' \
  "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-network-media-pair.out 2>&1; then
  fail "checker accepted network fallback verifier without media candidate-pair binding"
fi
grep -q "network fallback verifier must bind rendered media to selected candidate pair" /tmp/check-remoteapp-product-closure-network-media-pair.out || \
  fail "expected network fallback media candidate-pair binding failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh" "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"

perl -0pi -e 's#turn_relay#turn_model_only#g' \
  "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-network-turn.out 2>&1; then
  fail "checker accepted network fallback verifier without TURN relay coverage"
fi
grep -q "network fallback verifier must require TURN relay route evidence" /tmp/check-remoteapp-product-closure-network-turn.out || \
  fail "expected network fallback TURN relay failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh" "$SB/tools/scripts/remoteapp-network-fallback-e2e.sh"

perl -0pi -e 's#EASYNET_REMOTEAPP_BROWSER_ICE_TRANSPORT_POLICY=relay#EASYNET_REMOTEAPP_BROWSER_ICE_TRANSPORT_POLICY=all#g' \
  "$SB/tools/scripts/host-remoteapp-turn-relay-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-turn-relay-policy.out 2>&1; then
  fail "checker accepted a TURN host runner without relay-only Browser policy"
fi
grep -q "TURN relay host runner must constrain the real Browser to relay-only ICE" /tmp/check-remoteapp-product-closure-turn-relay-policy.out || \
  fail "expected TURN relay-only Browser policy failure"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-turn-relay-e2e.sh" "$SB/tools/scripts/host-remoteapp-turn-relay-e2e.sh"

perl -0pi -e 's#--route-kind easynet_relay#--route-kind turn_relay#g' \
  "$SB/tools/scripts/host-remoteapp-easynet-relay-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-easynet-relay-route.out 2>&1; then
  fail "checker accepted an EasyNet relay runner without focused route projection"
fi
grep -q "must project a focused EasyNet relay proof" /tmp/check-remoteapp-product-closure-easynet-relay-route.out || \
  fail "expected focused EasyNet relay projection failure"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-easynet-relay-e2e.sh" "$SB/tools/scripts/host-remoteapp-easynet-relay-e2e.sh"

perl -0pi -e 's#--release-probe#--release-observation-removed#g' \
  "$SB/tools/scripts/host-remoteapp-easynet-relay-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-easynet-relay-release.out 2>&1; then
  fail "checker accepted an EasyNet relay runner without terminal Hub cleanup proof"
fi
grep -q "must prove terminal Hub lease release" /tmp/check-remoteapp-product-closure-easynet-relay-release.out || \
  fail "expected EasyNet relay terminal release proof failure"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-easynet-relay-e2e.sh" "$SB/tools/scripts/host-remoteapp-easynet-relay-e2e.sh"

perl -0pi -e 's#-u EASYNET_REMOTE_DESKTOP_STUN_URLS#-u EASYNET_REMOTE_DESKTOP_UNUSED_STUN#g' \
  "$SB/tools/scripts/host-remoteapp-direct-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-direct-no-stun.out 2>&1; then
  fail "checker accepted a direct-route host runner that retained STUN configuration"
fi
grep -q "direct-route host runner must remove daemon STUN configuration" /tmp/check-remoteapp-product-closure-direct-no-stun.out || \
  fail "expected direct-route STUN-removal failure"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-direct-e2e.sh" "$SB/tools/scripts/host-remoteapp-direct-e2e.sh"

perl -0pi -e 's#daemon_zero_ice_servers_plus_host_only_sdp#selected_pair_happened_to_be_host#g' \
  "$SB/tools/scripts/project-remoteapp-network-scenario.py"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-direct-host-only.out 2>&1; then
  fail "checker accepted a direct projection without zero-ICE/host-only constraints"
fi
grep -q "network scenario projector must bind direct proof to zero ICE URLs and host-only SDP" /tmp/check-remoteapp-product-closure-direct-host-only.out || \
  fail "expected direct-route constraint projection failure"
cp "$REPO_ROOT/tools/scripts/project-remoteapp-network-scenario.py" "$SB/tools/scripts/project-remoteapp-network-scenario.py"

perl -0pi -e 's#EASYNET_REMOTEAPP_BROWSER_ALLOWED_OUTBOUND_ICE_CANDIDATE_TYPES=srflx,prflx#EASYNET_REMOTEAPP_BROWSER_ALLOWED_OUTBOUND_ICE_CANDIDATE_TYPES=host,srflx,prflx#g' \
  "$SB/tools/scripts/host-remoteapp-stun-srflx-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-stun-reflexive-only.out 2>&1; then
  fail "checker accepted a STUN host runner that admitted direct candidates"
fi
grep -q "STUN srflx host runner must block Browser host candidates on outbound signaling" /tmp/check-remoteapp-product-closure-stun-reflexive-only.out || \
  fail "expected STUN outbound Browser-host admission failure"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-stun-srflx-e2e.sh" "$SB/tools/scripts/host-remoteapp-stun-srflx-e2e.sh"

perl -0pi -e "s#\\QadmittedDescription('outbound', super.localDescription)\\E#super.localDescription#g" \
  "$SB/tools/scripts/host-remoteapp-stun-srflx-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-stun-local-sdp.out 2>&1; then
  fail "checker accepted a STUN runner without a Browser local-SDP admission guard"
fi
grep -q "must reject Browser code that leaks host candidates through local SDP" \
  /tmp/check-remoteapp-product-closure-stun-local-sdp.out || \
  fail "expected STUN Browser local-SDP admission failure"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-stun-srflx-e2e.sh" "$SB/tools/scripts/host-remoteapp-stun-srflx-e2e.sh"

perl -0pi -e 's#incoming packet BINDING processed, success#binding assumed from selected pair#g' \
  "$SB/tools/scripts/project-remoteapp-network-scenario.py"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-stun-server-binding.out 2>&1; then
  fail "checker accepted a STUN projector without server binding evidence"
fi
grep -q "network scenario projector must parse a server-observed STUN binding" /tmp/check-remoteapp-product-closure-stun-server-binding.out || \
  fail "expected STUN server-binding projection failure"
cp "$REPO_ROOT/tools/scripts/project-remoteapp-network-scenario.py" "$SB/tools/scripts/project-remoteapp-network-scenario.py"

perl -0pi -e 's#Global turn allocation count incremented#Relay path guessed from Browser stats#g' \
  "$SB/tools/scripts/project-remoteapp-network-scenario.py"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-turn-server-allocation.out 2>&1; then
  fail "checker accepted a TURN projector without server allocation evidence"
fi
grep -q "network scenario projector must parse a server-observed TURN allocation" /tmp/check-remoteapp-product-closure-turn-server-allocation.out || \
  fail "expected TURN server-allocation projection failure"
cp "$REPO_ROOT/tools/scripts/project-remoteapp-network-scenario.py" "$SB/tools/scripts/project-remoteapp-network-scenario.py"

perl -0pi -e 's#real_browser_tauri_lifecycle#component_mock_lifecycle#g' \
  "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-browser-lifecycle-mode.out 2>&1; then
  fail "checker accepted Browser/Tauri lifecycle verifier without real lifecycle proof mode"
fi
grep -q "frontend Browser/Tauri lifecycle verifier must require real lifecycle proof mode" /tmp/check-remoteapp-product-closure-browser-lifecycle-mode.out || \
  fail "expected Browser/Tauri lifecycle proof-mode failure"
cp "$REPO_ROOT/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh" "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"

perl -0pi -e 's#rtc_connection_state#rtc_state#g' \
  "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-browser-lifecycle-rtc.out 2>&1; then
  fail "checker accepted Browser/Tauri lifecycle verifier without connected WebRTC state"
fi
grep -q "frontend Browser/Tauri lifecycle verifier must require connected WebRTC state" /tmp/check-remoteapp-product-closure-browser-lifecycle-rtc.out || \
  fail "expected Browser/Tauri connected WebRTC state failure"
cp "$REPO_ROOT/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh" "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"

perl -0pi -e 's#browser_automation#component_state#g' \
  "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-browser-lifecycle-source.out 2>&1; then
  fail "checker accepted Browser/Tauri lifecycle verifier without UI automation evidence source"
fi
grep -q "frontend Browser/Tauri lifecycle verifier must require real UI automation evidence source" /tmp/check-remoteapp-product-closure-browser-lifecycle-source.out || \
  fail "expected Browser/Tauri evidence-source failure"
cp "$REPO_ROOT/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh" "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"

perl -0pi -e 's#observed_at_ms must be strictly increasing#observed_at_ms may repeat#g' \
  "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-browser-lifecycle-observed-order.out 2>&1; then
  fail "checker accepted Browser/Tauri lifecycle verifier without monotonic observed step timestamps"
fi
grep -q "frontend Browser/Tauri lifecycle verifier must require monotonic observed step timestamps" /tmp/check-remoteapp-product-closure-browser-lifecycle-observed-order.out || \
  fail "expected Browser/Tauri observed step order failure"
cp "$REPO_ROOT/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh" "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"

perl -0pi -e 's#frames_presented#frame_count#g' \
  "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-browser-lifecycle-frames.out 2>&1; then
  fail "checker accepted Browser/Tauri lifecycle verifier without rendered frame count"
fi
grep -q "frontend Browser/Tauri lifecycle verifier must require rendered frame count evidence" /tmp/check-remoteapp-product-closure-browser-lifecycle-frames.out || \
  fail "expected Browser/Tauri rendered frame count failure"
cp "$REPO_ROOT/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh" "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"

perl -0pi -e 's#terminal_receipt_visible#terminal_receipt_hidden#g' \
  "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-browser-lifecycle-terminal.out 2>&1; then
  fail "checker accepted Browser/Tauri lifecycle verifier without terminal receipt visibility"
fi
grep -q "frontend Browser/Tauri lifecycle verifier must inspect terminal receipt visibility" /tmp/check-remoteapp-product-closure-browser-lifecycle-terminal.out || \
  fail "expected Browser/Tauri terminal receipt visibility failure"
cp "$REPO_ROOT/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh" "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"

perl -0pi -e 's#input_applied target_focus_epoch must be positive#input_applied target_focus_epoch may be absent#g' \
  "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-browser-lifecycle-focus-positive.out 2>&1; then
  fail "checker accepted Browser/Tauri lifecycle verifier without positive focus epoch requirement"
fi
grep -q "frontend Browser/Tauri lifecycle verifier must require applied input focus epoch" /tmp/check-remoteapp-product-closure-browser-lifecycle-focus-positive.out || \
  fail "expected Browser/Tauri applied input focus epoch failure"
cp "$REPO_ROOT/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh" "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"

perl -0pi -e 's#submitted_frame target_focus_epoch must match input_applied target_focus_epoch#submitted_frame target_focus_epoch may differ#g' \
  "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-browser-lifecycle-submitted-focus.out 2>&1; then
  fail "checker accepted Browser/Tauri lifecycle verifier without submitted frame focus-epoch binding"
fi
grep -q "frontend Browser/Tauri lifecycle verifier must bind submitted input frame to target focus epoch" /tmp/check-remoteapp-product-closure-browser-lifecycle-submitted-focus.out || \
  fail "expected Browser/Tauri submitted frame focus-epoch binding failure"
cp "$REPO_ROOT/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh" "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"

perl -0pi -e 's#applied_event target_focus_epoch must match input_applied target_focus_epoch#applied_event target_focus_epoch may differ#g' \
  "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-browser-lifecycle-applied-focus.out 2>&1; then
  fail "checker accepted Browser/Tauri lifecycle verifier without daemon applied-event focus-epoch binding"
fi
grep -q "frontend Browser/Tauri lifecycle verifier must bind daemon applied event to target focus epoch" /tmp/check-remoteapp-product-closure-browser-lifecycle-applied-focus.out || \
  fail "expected Browser/Tauri daemon applied-event focus-epoch binding failure"
cp "$REPO_ROOT/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh" "$SB/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"

perl -0pi -e 's#remote_desktop\.show_session#remote_desktop.status#g' \
  "$SB/tools/scripts/host-remoteapp-session-timeout-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-timeout-show.out 2>&1; then
  fail "checker accepted session timeout E2E without public show_session observation"
fi
grep -q "session timeout E2E must observe timeout through public show_session" /tmp/check-remoteapp-product-closure-timeout-show.out || \
  fail "expected timeout show_session failure"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-session-timeout-e2e.sh" "$SB/tools/scripts/host-remoteapp-session-timeout-e2e.sh"

perl -0pi -e 's#terminal_receipt\.reason_code#terminal_receipt.reason#g' \
  "$SB/tools/scripts/host-remoteapp-session-timeout-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-timeout-receipt.out 2>&1; then
  fail "checker accepted session timeout E2E without terminal_receipt.reason_code validation"
fi
grep -q "session timeout E2E must inspect timeout terminal_receipt.reason_code" /tmp/check-remoteapp-product-closure-timeout-receipt.out || \
  fail "expected timeout receipt reason_code failure"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-session-timeout-e2e.sh" "$SB/tools/scripts/host-remoteapp-session-timeout-e2e.sh"

perl -0pi -e 's#remote_desktop\.end_session#remote_desktop.close_session#g' \
  "$SB/tools/scripts/host-remoteapp-session-cancel-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-cancel-end.out 2>&1; then
  fail "checker accepted session cancel E2E without public end_session invocation"
fi
grep -q "session cancel E2E must invoke public end_session" /tmp/check-remoteapp-product-closure-cancel-end.out || \
  fail "expected cancel end_session failure"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-session-cancel-e2e.sh" "$SB/tools/scripts/host-remoteapp-session-cancel-e2e.sh"

perl -0pi -e 's#terminal_receipt\.reason_code#terminal_receipt.reason#g' \
  "$SB/tools/scripts/host-remoteapp-session-cancel-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-cancel-receipt.out 2>&1; then
  fail "checker accepted session cancel E2E without terminal_receipt.reason_code validation"
fi
grep -q "session cancel E2E must inspect cancel terminal_receipt.reason_code" /tmp/check-remoteapp-product-closure-cancel-receipt.out || \
  fail "expected cancel receipt reason_code failure"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-session-cancel-e2e.sh" "$SB/tools/scripts/host-remoteapp-session-cancel-e2e.sh"

perl -0pi -e 's#real_platform_permission_revoke#synthetic_permission_revoke#g' \
  "$SB/tools/scripts/host-remoteapp-permission-revoke-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-permission-revoke-mode.out 2>&1; then
  fail "checker accepted permission revoke E2E without real platform proof mode"
fi
grep -q "permission revoke E2E must require real platform revoke proof mode" /tmp/check-remoteapp-product-closure-permission-revoke-mode.out || \
  fail "expected permission revoke proof-mode failure"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-permission-revoke-e2e.sh" "$SB/tools/scripts/host-remoteapp-permission-revoke-e2e.sh"

perl -0pi -e 's#target_permission_revoked#target_permission_suspended#g' \
  "$SB/tools/scripts/host-remoteapp-permission-revoke-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-permission-revoke-reason.out 2>&1; then
  fail "checker accepted permission revoke E2E without target_permission_revoked terminal reason"
fi
grep -q "permission revoke E2E must prove target_permission_revoked terminal reason" /tmp/check-remoteapp-product-closure-permission-revoke-reason.out || \
  fail "expected permission revoke terminal reason failure"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-permission-revoke-e2e.sh" "$SB/tools/scripts/host-remoteapp-permission-revoke-e2e.sh"

perl -0pi -e 's#remote_desktop\.refresh_lease#remote_desktop.recreate_session#g' \
  "$SB/tools/scripts/host-remoteapp-session-resume-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-session-resume-refresh.out 2>&1; then
  fail "checker accepted session resume E2E without public refresh_lease"
fi
grep -q "session resume E2E must invoke public refresh_lease" /tmp/check-remoteapp-product-closure-session-resume-refresh.out || \
  fail "expected session resume refresh_lease failure"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-session-resume-e2e.sh" "$SB/tools/scripts/host-remoteapp-session-resume-e2e.sh"

perl -0pi -e 's#show_after_original_lease must prove the refreshed session survived#show_after_original_lease may create a replacement session#g' \
  "$SB/tools/scripts/host-remoteapp-session-resume-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-session-resume-survival.out 2>&1; then
  fail "checker accepted session resume E2E without same-session survival proof"
fi
grep -q "session resume E2E must prove same-session survival after original lease" /tmp/check-remoteapp-product-closure-session-resume-survival.out || \
  fail "expected session resume survival failure"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-session-resume-e2e.sh" "$SB/tools/scripts/host-remoteapp-session-resume-e2e.sh"

perl -0pi -e 's#real_crash_restart_recovery_matrix#source_only_crash_restart_matrix#g' \
  "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-crash-proof-mode.out 2>&1; then
  fail "checker accepted crash/restart verifier without real recovery proof mode"
fi
grep -q "crash/restart recovery verifier must require real recovery proof mode" /tmp/check-remoteapp-product-closure-crash-proof-mode.out || \
  fail "expected crash/restart proof-mode failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh" "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"

perl -0pi -e 's#replay_guard_recovered must be true#replay_guard_recovered may be false#g' \
  "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-crash-replay-guard.out 2>&1; then
  fail "checker accepted crash/restart verifier without replay guard recovery"
fi
grep -q "crash/restart recovery verifier must require replay guard recovery" /tmp/check-remoteapp-product-closure-crash-replay-guard.out || \
  fail "expected crash/restart replay-guard failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh" "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"

perl -0pi -e 's#scenario_started_at_ms must be recorded#scenario start timestamp is optional#g' \
  "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-crash-scenario-start.out 2>&1; then
  fail "checker accepted crash/restart verifier without scenario start timestamp evidence"
fi
grep -q "crash/restart recovery verifier must require scenario start timestamp evidence" /tmp/check-remoteapp-product-closure-crash-scenario-start.out || \
  fail "expected crash/restart scenario start timestamp failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh" "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"

perl -0pi -e 's#events must be strictly ordered by at_ms#events may be unordered by at_ms#g' \
  "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-crash-event-order.out 2>&1; then
  fail "checker accepted crash/restart verifier without ordered lifecycle events"
fi
grep -q "crash/restart recovery verifier must require ordered lifecycle events" /tmp/check-remoteapp-product-closure-crash-event-order.out || \
  fail "expected crash/restart lifecycle event order failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh" "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"

perl -0pi -e 's#PROCESS_STOPPED_UNCLEAN must occur before DAEMON_RESTARTED#DAEMON_RESTARTED may occur before PROCESS_STOPPED_UNCLEAN#g' \
  "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-crash-daemon-order.out 2>&1; then
  fail "checker accepted crash/restart verifier without daemon restart ordering"
fi
grep -q "crash/restart recovery verifier must require daemon restart event ordering" /tmp/check-remoteapp-product-closure-crash-daemon-order.out || \
  fail "expected crash/restart daemon restart ordering failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh" "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"

perl -0pi -e 's#must remain stable across restart#may be replaced across restart#g' \
  "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-crash-session-stability.out 2>&1; then
  fail "checker accepted crash/restart verifier without stable public session requirement"
fi
grep -q "crash/restart recovery verifier must reject public session replacement" /tmp/check-remoteapp-product-closure-crash-session-stability.out || \
  fail "expected crash/restart session-stability failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh" "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"

perl -0pi -e 's#first_frame_rendered_after_restart_at_ms must be after media_reattached_at_ms#first rendered frame may precede media reattach#g' \
  "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-crash-frame-order.out 2>&1; then
  fail "checker accepted crash/restart verifier without rendered-after-media ordering"
fi
grep -q "crash/restart recovery verifier must require rendered media after media reattachment" /tmp/check-remoteapp-product-closure-crash-frame-order.out || \
  fail "expected crash/restart rendered-after-media ordering failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh" "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"

perl -0pi -e 's#terminal event identity must be replayed#terminal event identity may be replaced#g' \
  "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-crash-terminal-replay.out 2>&1; then
  fail "checker accepted crash/restart verifier without terminal receipt replay"
fi
grep -q "crash/restart recovery verifier must require original terminal event identity replay" /tmp/check-remoteapp-product-closure-crash-terminal-replay.out || \
  fail "expected crash/restart terminal-replay failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh" "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"

perl -0pi -e 's#show_session_after_restart_observed_at_ms must be after TERMINAL_RECEIPT_REPLAYED#show_session may precede terminal receipt replay#g' \
  "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-crash-show-after-replay.out 2>&1; then
  fail "checker accepted crash/restart verifier without show_session-after-replay timing"
fi
grep -q "crash/restart recovery verifier must require public show_session after receipt replay" /tmp/check-remoteapp-product-closure-crash-show-after-replay.out || \
  fail "expected crash/restart show-session-after-replay failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh" "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"

perl -0pi -e 's#endpoint_ready_at_ms must be after DAEMON_READY_AFTER_RESTART#endpoint readiness may precede daemon ready#g' \
  "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-crash-endpoint-order.out 2>&1; then
  fail "checker accepted crash/restart verifier without endpoint-after-ready timing"
fi
grep -q "crash/restart recovery verifier must require endpoint readiness after daemon-ready event" /tmp/check-remoteapp-product-closure-crash-endpoint-order.out || \
  fail "expected crash/restart endpoint-after-ready failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh" "$SB/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"

python3 - "$SB/docs/design/remoteapp-product-readiness-matrix.json" <<'PY'
import json
import sys

path = sys.argv[1]
matrix = json.load(open(path, encoding="utf-8"))
matrix["product_complete"] = True
matrix["status"] = "complete"
json.dump(matrix, open(path, "w", encoding="utf-8"), indent=2)
PY
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-matrix-complete.out 2>&1; then
  fail "checker accepted matrix that claims product completion"
fi
grep -q "product_complete must be false" /tmp/check-remoteapp-product-closure-matrix-complete.out || \
  fail "expected product_complete matrix failure"

cp "$REPO_ROOT/docs/design/remoteapp-product-readiness-matrix.json" "$SB/docs/design/remoteapp-product-readiness-matrix.json"
python3 - "$SB/docs/design/remoteapp-product-readiness-matrix.json" <<'PY'
import json
import sys

path = sys.argv[1]
matrix = json.load(open(path, encoding="utf-8"))
matrix["requirements"] = [
    row for row in matrix["requirements"] if row["id"] != "frontend_lifecycle"
]
json.dump(matrix, open(path, "w", encoding="utf-8"), indent=2)
PY
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-matrix-missing.out 2>&1; then
  fail "checker accepted matrix without frontend lifecycle row"
fi
grep -q "missing requirement ids: frontend_lifecycle" /tmp/check-remoteapp-product-closure-matrix-missing.out || \
  fail "expected missing frontend lifecycle matrix failure"

cp "$REPO_ROOT/docs/design/remoteapp-product-readiness-matrix.json" "$SB/docs/design/remoteapp-product-readiness-matrix.json"
perl -0pi -e 's/"terminal_receipt": session\.terminal_receipt\(\),//' \
  "$SB/plugins/remote-desktop/src/view.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-terminal-view.out 2>&1; then
  fail "checker accepted session view without terminal_receipt projection"
fi
grep -q "session view must expose terminal_receipt" /tmp/check-remoteapp-product-closure-terminal-view.out || \
  fail "expected missing terminal_receipt view failure"

cp "$REPO_ROOT/plugins/remote-desktop/src/view.rs" "$SB/plugins/remote-desktop/src/view.rs"
perl -0pi -e 's/idempotent end_session must return the original terminal receipt/idempotent end_session does not check terminal receipt/' \
  "$SB/plugins/remote-desktop/src/handlers/mod.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-terminal-idempotent.out 2>&1; then
  fail "checker accepted end_session without terminal receipt idempotency proof"
fi
grep -q "end_session tests must prove idempotent close returns the original terminal receipt" /tmp/check-remoteapp-product-closure-terminal-idempotent.out || \
  fail "expected missing terminal receipt idempotency failure"

perl -0pi -e 's#worker-only recovery must preserve daemon pid#worker-only recovery may replace daemon pid#g' \
  "$SB/tools/scripts/host-remoteapp-target-monitor-worker-recovery-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-worker-pid.out 2>&1; then
  fail "checker accepted target-monitor worker recovery that may replace the daemon"
fi
grep -q "target-monitor worker runner must require one stable daemon process" /tmp/check-remoteapp-product-closure-worker-pid.out || \
  fail "expected target-monitor worker stable daemon failure"

perl -0pi -e 's#trait RemoteDesktopRelayLeaseProvider#trait RemovedRelayLeaseProvider#g' \
  "$SB/plugins/remote-desktop/src/relay_lease.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-relay-port.out 2>&1; then
  fail "checker accepted RemoteApp relay acquisition without an injected product port"
fi
grep -q "relay acquisition must remain an injected product port" /tmp/check-remoteapp-product-closure-relay-port.out || \
  fail "expected injected RemoteApp relay port failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/relay_lease.rs" "$SB/plugins/remote-desktop/src/relay_lease.rs"

perl -0pi -e 's#load_credentials_optional#load_credentials_outside_daemon_boundary#g' \
  "$SB/src/daemon/plugins/remoteapp_relay.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-relay-credential-owner.out 2>&1; then
  fail "checker accepted a relay adapter outside the daemon credential boundary"
fi
grep -q "only the daemon relay adapter may load the durable device credential" /tmp/check-remoteapp-product-closure-relay-credential-owner.out || \
  fail "expected daemon relay credential-owner failure"
cp "$REPO_ROOT/src/daemon/plugins/remoteapp_relay.rs" "$SB/src/daemon/plugins/remoteapp_relay.rs"

perl -0pi -e 's#schedule_relay_refresh#removed_relay_scheduler#g' \
  "$SB/plugins/remote-desktop/src/lease_monitor.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-relay-refresh-owner.out 2>&1; then
  fail "checker accepted relay refresh outside the RemoteApp lease state machine"
fi
grep -q "relay refresh must remain in the single RemoteApp lease state machine" /tmp/check-remoteapp-product-closure-relay-refresh-owner.out || \
  fail "expected relay refresh state-machine failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/lease_monitor.rs" "$SB/plugins/remote-desktop/src/lease_monitor.rs"

perl -0pi -e 's#release_terminal_relay_lease#terminal_relay_release_removed#g' \
  "$SB/plugins/remote-desktop/src/session_lifecycle.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-relay-terminal-release.out 2>&1; then
  fail "checker accepted terminal settlement without relay release ownership"
fi
grep -q "terminal RemoteApp settlement must release" /tmp/check-remoteapp-product-closure-relay-terminal-release.out || \
  fail "expected terminal relay release failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/session_lifecycle.rs" "$SB/plugins/remote-desktop/src/session_lifecycle.rs"

perl -0pi -e 's#hub_relay_lease_reaches_both_ice_views_and_releases_after_terminal_commit#hub_relay_contract_not_proven#g' \
  "$SB/plugins/remote-desktop/src/handlers/end_session.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-relay-shared-lease.out 2>&1; then
  fail "checker accepted relay implementation without shared Browser/device and release proof"
fi
grep -q "shared Browser/device relay configuration" /tmp/check-remoteapp-product-closure-relay-shared-lease.out || \
  fail "expected shared relay lease proof failure"
cp "$REPO_ROOT/plugins/remote-desktop/src/handlers/end_session.rs" "$SB/plugins/remote-desktop/src/handlers/end_session.rs"

echo "test_check_remoteapp_product_closure_audit: ok"
