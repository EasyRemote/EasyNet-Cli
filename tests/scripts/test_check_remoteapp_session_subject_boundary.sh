#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-session-subject-boundary.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

mkdir -p "$SANDBOX/plugins/remote-desktop/src" "$SANDBOX/plugins/remote-desktop/abilities"

cat >"$SANDBOX/plugins/remote-desktop/src/session_access.rs" <<'RS'
fn ensure_session_control_identity(ability: &'static str, env: EnvelopeContext, args: Value, session: Session) {
    reject_subject_in_args(ability, args);
    ensure_session_subject_consistent(ability, env.subject(), session);
}

#[test]
fn session_control_subject_contract_is_original_resource_ura_not_session_ura() {}

#[test]
fn session_control_rejects_subject_in_args_even_when_token_matches() {}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/view.rs" <<'RS'
fn serialize(session: Session) -> Value {
    json!({
        "session_id": session.session_id(),
        "subject_ura": session.subject_ura(),
    })
}
RS

for ability in show_session set_description add_ice_candidate refresh_lease watch_events end_session; do
  cat >"$SANDBOX/plugins/remote-desktop/abilities/remote_desktop.${ability}.ability.toml" <<'TOML'
[input_schema]
type = "object"
TOML
done

CHECK_REMOTEAPP_SESSION_SUBJECT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

perl -0pi -e 's/ensure_session_subject_consistent\(ability, env\.subject\(\), session\);//' \
  "$SANDBOX/plugins/remote-desktop/src/session_access.rs"
if CHECK_REMOTEAPP_SESSION_SUBJECT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp session subject checker accepted missing envelope subject comparison" >&2
  exit 1
fi
perl -0pi -e 's/reject_subject_in_args\(ability, args\);/reject_subject_in_args(ability, args);\\n    ensure_session_subject_consistent(ability, env.subject(), session);/' \
  "$SANDBOX/plugins/remote-desktop/src/session_access.rs"

cat >>"$SANDBOX/plugins/remote-desktop/src/handler.rs" <<'RS'
fn bad_subject() {
    let subject = "easynet:///r/acme/resource/remote-desktop-session/rd-1";
}
RS
if CHECK_REMOTEAPP_SESSION_SUBJECT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp session subject checker accepted synthetic session URA subject" >&2
  exit 1
fi

echo "test_check_remoteapp_session_subject_boundary.sh: all cases passed"
