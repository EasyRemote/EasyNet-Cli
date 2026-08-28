#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-target-binding-boundary.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

fail() {
  printf 'test_check_remoteapp_target_binding_boundary: %s\n' "$1" >&2
  exit 1
}

fresh_fixture() {
  rm -rf "$SANDBOX"
  mkdir -p \
    "$SANDBOX/plugins" \
    "$SANDBOX/docs/design" \
    "$SANDBOX/src/daemon/ability/builtins/resources/media"
  cp -R "$REPO_ROOT/plugins/remote-desktop" "$SANDBOX/plugins/"
  cp "$REPO_ROOT/docs/design/remoteapp-targeted-session-spec.md" \
    "$SANDBOX/docs/design/remoteapp-targeted-session-spec.md"
  cp "$REPO_ROOT/src/daemon/ability/builtins/resources/media/screen_snapshot.rs" \
    "$SANDBOX/src/daemon/ability/builtins/resources/media/screen_snapshot.rs"
}

run_gate() {
  CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null
}

expect_fail() {
  local expected="$1"
  local output
  output="$(mktemp "${TMPDIR:-/tmp}/remoteapp-target-binding-mutation.XXXXXX")"
  if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >"$output" 2>&1; then
    rm -f "$output"
    fail "checker accepted mutation: $expected"
  fi
  if ! rg -q -- "$expected" "$output"; then
    sed -n '1,120p' "$output" >&2
    rm -f "$output"
    fail "checker rejected mutation for the wrong reason; expected: $expected"
  fi
  rm -f "$output"
}

fresh_fixture
run_gate

fresh_fixture
touch "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_native_media.rs"
expect_fail 'obsolete daemon-local media implementation remains'

fresh_fixture
perl -0pi -e 's/let target = target_plan\(binding\)\?;/let target = unbound_target_plan()?;/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs"
expect_fail 'hosted WebRTC media must derive its private host contract from RemoteAppTargetBinding'

fresh_fixture
perl -0pi -e 's/binding\.committed_app_window_set\(\)/binding.uncommitted_app_window_set()/g' \
  "$SANDBOX/plugins/remote-desktop/src/media_host_probe.rs"
expect_fail 'application host contracts must consume the committed AppWindowSetProof'

fresh_fixture
perl -0pi -e 's/binding\.committed_app_surface_layout\(\)/binding.uncommitted_app_surface_layout()/g' \
  "$SANDBOX/plugins/remote-desktop/src/media_host_probe.rs"
expect_fail 'application host contracts must consume the committed surface-layout proof'

fresh_fixture
perl -0pi -e 's/plan\.validate\(\)/plan.accept_without_validation()/g' \
  "$SANDBOX/plugins/remote-desktop/src/media_host_probe.rs"
expect_fail 'binding-derived media-host contracts must pass protocol validation before process start'

fresh_fixture
perl -0pi -e 's/proof\.validate_for\(plan\)\?;/let _ = plan;/' \
  "$SANDBOX/plugins/remote-desktop/src/media_host_probe.rs"
expect_fail 'media-host capture proofs must validate against the exact binding-derived contract'

fresh_fixture
perl -0pi -e 's/commit_pending_media_rebind_for_session/commit_pending_media_rebind_after_activation/g' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs"
expect_fail 'hosted WebRTC rebind must prepare and validate a replacement generation, commit Runtime binding, then activate media'

fresh_fixture
perl -0pi -e 's/observed_ids != expected_front_to_back/observed_ids == expected_front_to_back/' \
  "$SANDBOX/plugins/remote-desktop/media-host/src/macos_sck.rs"
expect_fail 'ScreenCaptureKit application capture must reject window order or membership drift'

fresh_fixture
perl -0pi -e 's/sorted_observed != contract\.window_ids/sorted_observed == contract.window_ids/' \
  "$SANDBOX/plugins/remote-desktop/media-host/src/macos_sck.rs"
expect_fail 'ScreenCaptureKit application capture must reject committed window-set drift'

fresh_fixture
perl -0pi -e 's/initWithDesktopIndependentWindow/initWithDisplay_excludingWindows/g' \
  "$SANDBOX/plugins/remote-desktop/media-host/src/macos_sck.rs"
expect_fail 'each committed macOS application window must use a desktop-independent ScreenCaptureKit filter'

fresh_fixture
perl -0pi -e 's/application surface membership differs from committed window set/application surface membership accepted without committed window set/' \
  "$SANDBOX/plugins/remote-desktop/native-protocol/src/media_session.rs"
expect_fail 'private media protocol must reject application layout membership'

fresh_fixture
perl -0pi -e 's/downcast_ref::<HostedMediaHostFailure>/downcast_ref::<String>/g' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_media.rs"
expect_fail 'WebRTC media failure projection must preserve typed media-host target failures'

fresh_fixture
cat >>"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs" <<'RS'
fn invalid_resource_reresolution(entry: ResourceEntry) {
    target_for_entry(entry);
}
RS
expect_fail 'production must not resolve native capture targets from ResourceEntry'

fresh_fixture
perl -0pi -e 's/EffectiveRemoteDesktopInputPolicy::for_binding/RemoteDesktopInputPolicy::default/g' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_negotiation.rs"
expect_fail 'WebRTC input policy must derive its typed execution policy from the session-owned RemoteAppTargetBinding'

fresh_fixture
perl -0pi -e 's/create_session_live_revalidates_an_expired_picker_row_before_insert/create_session_rejects_an_expired_picker_row_before_insert/g' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"
expect_fail 'expired picker cache must have device-side live-verification session-admission coverage'

fresh_fixture
perl -0pi -e 's/"live_identity_reverified"/"live_identity_assumed"/g' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"
expect_fail 'target diagnostics must expose committed live identity revalidation'

fresh_fixture
run_gate

printf 'test_check_remoteapp_target_binding_boundary: ok\n'
