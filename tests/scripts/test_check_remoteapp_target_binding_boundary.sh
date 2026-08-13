#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-target-binding-boundary.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

mkdir -p "$SANDBOX/plugins/remote-desktop/src/handlers"
mkdir -p "$SANDBOX/plugins/remote-desktop/src/transport"

cat >"$SANDBOX/plugins/remote-desktop/src/target.rs" <<'RS'
pub struct ResourceEntryTargetResolver;
RS

cat >"$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs" <<'RS'
fn create_session() {
    ResourceEntryTargetResolver.resolve_for_session();
    verify_target_binding_for_session();
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/handlers/attach.rs" <<'RS'
fn attach(session: Session) {
    let binding = session.target_binding();
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_negotiation.rs" <<'RS'
fn negotiate(session: Session) {
    let binding = session.target_binding().clone();
    input_policy_for_binding();
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_native_media.rs" <<'RS'
fn media(binding: Binding) {
    target_for_binding();
}
RS

CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

cat >>"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_native_media.rs" <<'RS'
fn bad(entry: ResourceEntry) {
    target_for_entry(entry);
}
RS

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted ResourceEntry native resolution" >&2
  exit 1
fi

perl -0pi -e 's/target_for_entry\(entry\);//' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_native_media.rs"
cat >>"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_negotiation.rs" <<'RS'
fn bad_resolver() {
    ResourceEntryTargetResolver.resolve_for_session();
}
RS

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted resolver use after session creation" >&2
  exit 1
fi

echo "test_check_remoteapp_target_binding_boundary.sh: all cases passed"
