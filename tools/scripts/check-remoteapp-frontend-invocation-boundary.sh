#!/usr/bin/env bash
set -euo pipefail

CLI_ROOT="${CHECK_REMOTEAPP_FRONTEND_CLI_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
FRONTEND_ROOT="${CHECK_REMOTEAPP_FRONTEND_ROOT:-$(cd "$CLI_ROOT/../EasyNet" && pwd)}"
FRONTEND_SRC="$FRONTEND_ROOT/Frontend/src"

fail() {
  printf 'check-remoteapp-frontend-invocation-boundary: %s\n' "$1" >&2
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

[[ -d "$FRONTEND_SRC" ]] || fail "missing EasyNet frontend source root"

INVOCATION="$FRONTEND_SRC/store/media-channel-invocation.ts"
STORE="$FRONTEND_SRC/store/media-channel-store.ts"
ACCESS="$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx"
INVOCATION_TEST="$FRONTEND_SRC/store/media-channel-invocation.test.ts"
STORE_TEST="$FRONTEND_SRC/store/media-channel-store.test.ts"
ACCESS_TEST="$FRONTEND_SRC/components/easynet/DeviceMediaAccess.test.tsx"
WORKSPACE="$FRONTEND_SRC/pages/easynet/DeviceMediaWorkspacePage.tsx"

for file in "$INVOCATION" "$STORE" "$ACCESS" "$INVOCATION_TEST" "$STORE_TEST" "$ACCESS_TEST" "$WORKSPACE"; do
  [[ -f "$file" ]] || fail "missing frontend source ${file#"$FRONTEND_ROOT/"}"
done

require 'REMOTE_DESKTOP_SESSION_SUBJECT_REQUIRED_ABILITIES' "$INVOCATION" \
  'frontend must maintain one explicit subject-required remote desktop ability set'
for ability in \
  remote_desktop.grant_consent \
  remote_desktop.create_session \
  remote_desktop.attach \
  remote_desktop.set_description \
  remote_desktop.add_ice_candidate \
  remote_desktop.report_client_state \
  remote_desktop.show_session \
  remote_desktop.refresh_lease \
  remote_desktop.watch_events \
  remote_desktop.end_session; do
  require "'$ability'" "$INVOCATION" \
    "frontend subject-required ability set must include $ability"
done

require 'requireRemoteDesktopSessionSubject\(ability, opts\.subjectURA\)' "$INVOCATION" \
  'frontend unary/stream media invocation paths must reject remote desktop calls without subjectURA'
require 'subject: opts\.subjectURA' "$INVOCATION" \
  'frontend invocation material must derive Invocation.subject from opts.subjectURA'
require "\? \{ kind: 'ura', ura: opts\.subjectURA \}" "$INVOCATION" \
  'frontend invocation material must encode selected target as a URA subject'
require ": \{ kind: 'authenticated-user' \}" "$INVOCATION" \
  'frontend authenticated-user fallback must remain outside subject-required remote desktop calls'
require 'remote desktop session subject_ura is required' "$INVOCATION" \
  'frontend must fail closed with an explicit missing remote desktop subject error'

require "invokeMediaUnaryResponse\\('remote_desktop\\.grant_consent'" "$STORE" \
  'frontend remote desktop creation must grant consent before session creation'
require "invokeMediaUnary\\('remote_desktop\\.create_session'" "$STORE" \
  'frontend remote desktop creation must call remote_desktop.create_session'
require 'subjectURA: resource\.resource_ura' "$STORE" \
  'frontend grant_consent/create_session must use the selected resource URA as Invocation.subject'
require 'assertRemoteDesktopCreateSessionIdentity\(result\)' "$STORE" \
  'frontend create_session response must be identity-checked before projection'
require 'remote_desktop\.create_session response did not include subject_ura' "$STORE" \
  'frontend create_session response must fail closed when subject_ura is missing'
require 'remoteDesktopConsentCausalContext\(consent\)' "$STORE" \
  'frontend create_session must causally chain to the consent receipt'
require 'consent_ticket: consentTicket' "$STORE" \
  'frontend create_session args must carry only daemon-issued consent ticket, not subject identity'

reject 'subject_ura:' "$STORE" \
  'frontend create_session args must not carry subject_ura; use Invocation.subject'
reject 'resource_ura: resource\.resource_ura' "$STORE" \
  'frontend create_session args must not carry resource_ura; use Invocation.subject'

require 'resource\.refresh_remote_targets' "$ACCESS" \
  'frontend display/application/window picker must use live resource.refresh_remote_targets'
require 'screenResource = screenResources\.find\(\(resource\) => resource\.resource_ura === selectedScreenURA\)' "$ACCESS" \
  'frontend selected target must be an exact selectedScreenURA match'
require 'selectedScreenURA && !screenResources\.some\(\(resource\) => resource\.resource_ura === selectedScreenURA\)' "$ACCESS" \
  'frontend selected target must clear stale selections instead of falling back'
reject 'screenResources\[0\]' "$ACCESS" \
  'frontend access dialog must not fall back to the first remote target'
require 'baseRuntimeReady' "$ACCESS" \
  'frontend must separate base media runtime readiness from remote target readiness'
require 'remoteTargetReady' "$ACCESS" \
  'frontend must gate only screen/remote desktop launchers on live target readiness'

require 'listRemoteDesktopTargets' "$WORKSPACE" \
  'frontend workspace must use live remote target inventory'
require 'refetchInterval: runtimeOnline \? 5000 : false' "$WORKSPACE" \
  'frontend workspace must continue refreshing remote targets while online'
require 'resource\.resource_ura === entry\.session\?\.subjectUra' "$WORKSPACE" \
  'frontend workspace must bind target display to the session subject'
reject 'screenResources\[0\]' "$WORKSPACE" \
  'frontend workspace must not fall back to the first remote target'
require 'Session target is no longer advertised by the live target inventory' "$WORKSPACE" \
  'frontend workspace must surface stale session target state'

require 'requires an explicit remote desktop subject' "$INVOCATION_TEST" \
  'frontend invocation tests must cover missing remote desktop subject failures'
require 'remote_desktop\.create_session' "$INVOCATION_TEST" \
  'frontend invocation tests must cover create_session subject propagation'
require 'subjectURA: .*streams/display-1' "$INVOCATION_TEST" \
  'frontend invocation tests must prove selected resource subject propagation'
require 'reports missing remote desktop session subject before projection fallback' "$STORE_TEST" \
  'frontend store tests must prove create_session subject_ura is checked before projection'
require 'keeps base media controls available when remote desktop target refresh fails' "$ACCESS_TEST" \
  'frontend access tests must prove remote target failure does not disable base media'

printf 'check-remoteapp-frontend-invocation-boundary: ok\n'
