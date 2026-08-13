#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_SESSION_SUBJECT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
REMOTE_ROOT="$ROOT/plugins/remote-desktop/src"
SESSION_ACCESS="$REMOTE_ROOT/session_access.rs"
VIEW="$REMOTE_ROOT/view.rs"

fail() {
  printf 'check-remoteapp-session-subject-boundary: %s\n' "$1" >&2
  exit 1
}

require() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  rg -q -- "$pattern" "$path" || fail "$message"
}

reject() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  if rg -q -- "$pattern" "$path"; then
    fail "$message"
  fi
}

[[ -f "$SESSION_ACCESS" ]] || fail "missing remote desktop session_access.rs"
[[ -f "$VIEW" ]] || fail "missing remote desktop view.rs"

require 'reject_subject_in_args\(ability, args\)' "$SESSION_ACCESS" \
  'session control must reject subject/resource_ura duplicated in ability args'
require 'ensure_session_subject_consistent\(ability, env\.subject\(\), session\)' "$SESSION_ACCESS" \
  'session control must compare Invocation.subject with the session resource subject'
require 'session_control_subject_contract_is_original_resource_ura_not_session_ura' "$SESSION_ACCESS" \
  'session subject contract test must reject session URA substitution'
require 'session_control_rejects_subject_in_args_even_when_token_matches' "$SESSION_ACCESS" \
  'session subject contract test must reject args.subject even with a valid token'
require '"subject_ura": session\.subject_ura\(\)' "$VIEW" \
  'session views must expose the original selected resource URA as subject_ura'
require '"session_id": session\.session_id\(\)' "$VIEW" \
  'session views must keep session_id as a session access identifier'

# Production remote-desktop code must not route lifecycle abilities by a
# synthetic remote-desktop-session resource subject. Session ids and tokens are
# access facts in args; the Invocation.subject remains the selected target
# resource URA captured by create_session.
while IFS=: read -r file line text; do
  relative="${file#"$ROOT/"}"
  case "$relative" in
    plugins/remote-desktop/src/session_access.rs|\
    tools/scripts/check-remoteapp-session-subject-boundary.sh|\
    tests/scripts/test_check_remoteapp_session_subject_boundary.sh)
      continue
      ;;
  esac
  if grep -q 'remote-desktop-session' <<<"$text"; then
    fail "$relative:$line must not introduce a synthetic session URA as remote desktop lifecycle subject"
  fi
done < <(rg -n -- 'remote-desktop-session' "$REMOTE_ROOT" || true)

reject '"subject"[[:space:]]*:' "$ROOT/plugins/remote-desktop/abilities/remote_desktop.show_session.ability.toml" \
  'show_session schema must not accept args.subject'
reject '"subject"[[:space:]]*:' "$ROOT/plugins/remote-desktop/abilities/remote_desktop.set_description.ability.toml" \
  'set_description schema must not accept args.subject'
reject '"subject"[[:space:]]*:' "$ROOT/plugins/remote-desktop/abilities/remote_desktop.add_ice_candidate.ability.toml" \
  'add_ice_candidate schema must not accept args.subject'
reject '"subject"[[:space:]]*:' "$ROOT/plugins/remote-desktop/abilities/remote_desktop.refresh_lease.ability.toml" \
  'refresh_lease schema must not accept args.subject'
reject '"subject"[[:space:]]*:' "$ROOT/plugins/remote-desktop/abilities/remote_desktop.watch_events.ability.toml" \
  'watch_events schema must not accept args.subject'
reject '"subject"[[:space:]]*:' "$ROOT/plugins/remote-desktop/abilities/remote_desktop.end_session.ability.toml" \
  'end_session schema must not accept args.subject'

printf 'check-remoteapp-session-subject-boundary: ok\n'
