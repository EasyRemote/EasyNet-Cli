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

require_multiline() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  perl -0ne "exit(($pattern) ? 0 : 1)" "$path" || fail "$message"
}

reject() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  if rg -q -- "$pattern" "$path"; then
    fail "$message"
  fi
}

reject_multiline() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  if perl -0ne "exit(($pattern) ? 0 : 1)" "$path"; then
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
PROTOCOL="$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"
PROTOCOL_TEST="$FRONTEND_SRC/lib/api/remote-desktop-protocol.test.ts"

for file in "$INVOCATION" "$STORE" "$ACCESS" "$INVOCATION_TEST" "$STORE_TEST" "$ACCESS_TEST" "$WORKSPACE" "$PROTOCOL" "$PROTOCOL_TEST"; do
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
for ability in \
  remote_desktop.permission_status \
  remote_desktop.request_permission; do
  reject "'$ability'" "$INVOCATION" \
    "frontend host-local permission probe $ability must not require a target resource subject"
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
require_multiline '/rdRequestPermission:[\s\S]*invokeMediaUnary\('\''remote_desktop\.request_permission'\''[\s\S]*deviceUra: entry\.deviceUra,[\s\S]*args: \{\},[\s\S]*\}\)/s' "$STORE" \
  'frontend request_permission must invoke the host-local permission probe without a target subjectURA'
reject_multiline '/remote_desktop\.request_permission(?:(?!\}\)).)*subjectURA:/s' "$STORE" \
  'frontend request_permission must not scope host-local permission probes to the selected remote desktop resource'
require "invokeMediaUnary\\('remote_desktop\\.report_client_state'" "$STORE" \
  'frontend must report browser/client media presentation through remote_desktop.report_client_state'
require 'subjectURA: currentView\.subjectUra' "$STORE" \
  'frontend client media report must use the session subject URA as Invocation.subject'
require 'transport_epoch: epoch' "$STORE" \
  'frontend client media report must bind presentation state to the negotiated transport epoch'
require 'state: desired' "$STORE" \
  'frontend client media report must submit the latest desired client media state'
require "clientMediaReportedState === 'presenting'" "$STORE" \
  'frontend presentation timeout must observe the reported client-presenting state'
require 'remote desktop did not present a frame within' "$STORE" \
  'frontend must fail closed when decoded-frame presentation is not observed'
reject_multiline '/pc\.connectionState === '\''connected'\''[\s\S]{0,240}reportClientMediaState\(key, '\''presenting'\''\)/s' "$STORE" \
  'frontend must not report production presentation from peer-connection connected alone'
require 'stopRemoteDesktopEventWatch' "$STORE" \
  'frontend must own remote desktop watch_events stream teardown'
require 'remoteDesktopEventsAbort\?\.abort\(\)' "$STORE" \
  'frontend must abort the remote desktop watch_events stream on close/end'
require 'startRemoteDesktopEventWatch\(key, negotiated\)' "$STORE" \
  'frontend must start watch_events after WebRTC session negotiation'
require_multiline 'm/invokeMediaStream\(\s*'\''remote_desktop\.watch_events'\''[\s\S]*subjectURA: view\.subjectUra[\s\S]*causalContext[\s\S]*args: \{ session_id: view\.sessionId, session_token: view\.sessionToken \}[\s\S]*timeoutMs: 0/s' "$STORE" \
  'frontend must subscribe to remote_desktop.watch_events with the negotiated session subject, causal context, and token'
require 'remoteDesktopSessionEventRecovery' "$STORE" \
  'frontend must map remote desktop session events into recovery UI state'
require 'SESSION_DEGRADED' "$STORE" \
  'frontend must surface degraded client/media sessions as retryable recovery state'
require 'TARGET_PERMISSION_REVOKED' "$STORE" \
  'frontend must surface permission revocation from watch_events'
require 'closeLocalTransport' "$STORE" \
  'frontend recovery mapping must explicitly decide whether to close local transport'
require_multiline 'm/TARGET_PERMISSION_REVOKED[\s\S]*closeLocalTransport:\s*true/s' "$STORE" \
  'frontend permission-revoked recovery must close local WebRTC/input transport'

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
require 'requestVideoFrameCallback' "$ACCESS" \
  'frontend WebRTC viewport must prefer decoded-frame callbacks for client-presenting evidence'
require 'if \(!videoWithFrameCallback\.requestVideoFrameCallback\) onPresented\(\)' "$ACCESS" \
  'frontend WebRTC viewport may use playing only as a no-requestVideoFrameCallback fallback'
require 'onPresented=\{reportPresented\}' "$ACCESS" \
  'frontend WebRTC viewport must wire decoded-frame presentation to client media reporting'
require "reportClientMediaState\\(channelKey, 'presenting'\\)" "$ACCESS" \
  'frontend media viewport must report presenting only through the remote desktop client media state action'

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

require 'productionReady: productionReadiness\?\.ready === true' "$PROTOCOL" \
  'frontend production online must derive from production_readiness.ready only'
reject 'productionReady: result\?\.production_media_ready === true \|\| productionReadiness\?\.ready === true' "$PROTOCOL" \
  'frontend must not OR legacy production_media_ready into the production online predicate'
reject 'productionReady: productionGate\?\.ready === true \|\| mediaBackends\.some\(isRemoteDesktopProductionBackend\)' "$PROTOCOL" \
  'frontend must not report production online from production_gate or backend availability alone'
require 'productionReadiness' "$PROTOCOL" \
  'frontend must preserve production_readiness evidence for remote desktop sessions'
require 'latestTargetDiagnostic' "$PROTOCOL" \
  'frontend must preserve latest_target_diagnostic evidence for remote desktop sessions'
require 'targetTracking' "$PROTOCOL" \
  'frontend must preserve target_tracking evidence for remote desktop sessions'
require 'frontendAction' "$PROTOCOL" \
  'frontend must expose target diagnostic frontend_action for recovery UI decisions'
require 'inputEnabled' "$PROTOCOL" \
  'frontend must expose target tracking input_enabled so target loss cannot appear interactive'
require 'remoteDesktopInputFrameAllowed' "$PROTOCOL" \
  'frontend must derive remote desktop input eligibility from runtime target tracking and input policy'
require 'remoteDesktopInputFrameAllowed' "$STORE" \
  'frontend store input sender must use the remote desktop input eligibility helper'
require_multiline 'm/rdSendInput:\s*\(key,\s*frame\)\s*=>\s*\{(?:(?!bindCanvas).)*remoteDesktopInputFrameAllowed\(session,\s*frame\)/s' "$STORE" \
  'frontend rdSendInput must check session target tracking/input policy before sending frames'
require 'remoteDesktopTargetRecoveryMessage' "$PROTOCOL" \
  'frontend must derive target-domain recovery messages from runtime target diagnostics'
require_multiline 'm/const reason = remoteDesktopTargetRecoveryMessage\(view\)\s*\?\?\s*view\.productionBlockedReason/s' "$PROTOCOL" \
  'frontend production blocked messages must prefer runtime target recovery diagnostics'

require 'requires an explicit remote desktop subject' "$INVOCATION_TEST" \
  'frontend invocation tests must cover missing remote desktop subject failures'
require 'remote_desktop\.create_session' "$INVOCATION_TEST" \
  'frontend invocation tests must cover create_session subject propagation'
require 'subjectURA: .*streams/display-1' "$INVOCATION_TEST" \
  'frontend invocation tests must prove selected resource subject propagation'
require 'reports missing remote desktop session subject before projection fallback' "$STORE_TEST" \
  'frontend store tests must prove create_session subject_ura is checked before projection'
require 'runs remote desktop WebRTC sessions with a target-scoped event watcher' "$STORE_TEST" \
  'frontend store tests must prove watch_events is bound to the negotiated session subject'
require 'surfaces remote desktop recovery events from the session watcher' "$STORE_TEST" \
  'frontend store tests must prove watch_events recovery events affect UI/transport state'
require 'projects target diagnostics and tracking state from the runtime session view' "$PROTOCOL_TEST" \
  'frontend protocol tests must cover latest target diagnostic and tracking projection'
require 'does not treat ordinary view-only input state as target recovery failure' "$PROTOCOL_TEST" \
  'frontend protocol tests must prove view-only input state is not misreported as target loss'
require 'gates frontend input frames on runtime target tracking and input policy' "$PROTOCOL_TEST" \
  'frontend protocol tests must cover target-tracking and input-policy input gating'
require 'keeps base media controls available when remote desktop target refresh fails' "$ACCESS_TEST" \
  'frontend access tests must prove remote target failure does not disable base media'
require 'runs the remote desktop UI flow from target picker through session end' "$ACCESS_TEST" \
  'frontend access tests must prove picker-to-session-to-end remote desktop UI flow'
require 'remote_desktop\.grant_consent' "$ACCESS_TEST" \
  'frontend access tests must prove UI flow grants target-scoped consent'
require 'remote_desktop\.create_session' "$ACCESS_TEST" \
  'frontend access tests must prove UI flow creates a remote desktop session'
require 'remote_desktop\.watch_events' "$ACCESS_TEST" \
  'frontend access tests must prove UI flow starts the watch_events stream'
require 'remote_desktop\.end_session' "$ACCESS_TEST" \
  'frontend access tests must prove UI flow ends the session'
require 'subject_ura: screenResource\.resource_ura' "$ACCESS_TEST" \
  'frontend access tests must assert selected target subject propagation through UI flow'

printf 'check-remoteapp-frontend-invocation-boundary: ok\n'
