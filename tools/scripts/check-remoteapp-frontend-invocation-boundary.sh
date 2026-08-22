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
require 'remoteDesktopSessionInputIntent\(entry\)' "$STORE" \
  'frontend grant_consent/create_session must share one remote desktop input intent object'
require 'inputControlRequested: boolean' "$STORE" \
  'frontend remote desktop input intent must model explicit input-control consent scope'
require 'input_control: sessionInputIntent\.inputControlRequested' "$STORE" \
  'frontend grant_consent must request input_control from the same session input intent used for create_session'
require 'mode: sessionInputIntent\.mode' "$STORE" \
  'frontend create_session mode must come from the shared remote desktop input intent'
require 'input_policy: sessionInputIntent\.inputPolicy' "$STORE" \
  'frontend create_session input_policy must come from the shared remote desktop input intent'
reject 'mode: entry\.interactive \?' "$STORE" \
  'frontend create_session mode must not independently derive from entry.interactive'
reject 'keyboard_enabled: entry\.interactive' "$STORE" \
  'frontend create_session keyboard policy must not independently derive from entry.interactive'
reject 'pointer_enabled: entry\.interactive' "$STORE" \
  'frontend create_session pointer policy must not independently derive from entry.interactive'
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
require 'syncRemoteDesktopTerminalSession' "$STORE" \
  'frontend must synchronize daemon terminal session projection after terminal recovery events'
require "invokeMediaUnary\\('remote_desktop\\.show_session'" "$STORE" \
  'frontend terminal recovery sync must read the daemon session view through remote_desktop.show_session'
require 'syncTerminalSession: true' "$STORE" \
  'frontend permission-revoked recovery must request daemon terminal session synchronization'
require 'terminal sync failed' "$STORE" \
  'frontend terminal recovery sync failures must remain visible to the operator'

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
require 'targetGeometryRevision = entry\.session\?\.targetTracking\?\.targetGeometryRevision' "$ACCESS" \
  'frontend pointer input frames must bind to the session target geometry revision when available'
require 'target_geometry_revision: targetGeometryRevision' "$ACCESS" \
  'frontend pointer input frames must carry target_geometry_revision for daemon stale-transform rejection'
require 'remoteDesktopInputReadinessLabel\(session\)' "$ACCESS" \
  'frontend session details must render daemon input_readiness instead of only requested input policy'
require 'const readiness = view\.inputReadiness' "$ACCESS" \
  'frontend input readiness details must read the daemon-projected inputReadiness object'
require 'readiness\.interactiveReady' "$ACCESS" \
  'frontend input readiness details must expose daemon interactive_ready state'
require 'readiness\.blockedReason' "$ACCESS" \
  'frontend input readiness details must expose daemon blocked_reason state'

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
require 'RemoteDesktopInputReadiness' "$PROTOCOL" \
  'frontend must model daemon-projected remote desktop input readiness'
require 'inputReadiness\?: RemoteDesktopInputReadiness' "$PROTOCOL" \
  'frontend session view must carry the daemon input_readiness projection'
require 'RemoteDesktopTerminalReceipt' "$PROTOCOL" \
  'frontend must model daemon-projected remote desktop terminal receipts'
require 'terminalReceipt\?: RemoteDesktopTerminalReceipt' "$PROTOCOL" \
  'frontend session view must carry the daemon terminal_receipt projection'
require 'remoteDesktopInputReadinessFromResult\(result\)' "$PROTOCOL" \
  'frontend session projection must parse daemon input_readiness'
require 'remoteDesktopTerminalReceiptFromResult\(result\)' "$PROTOCOL" \
  'frontend session projection must parse daemon terminal_receipt'
require 'objectField\(result, '\''input_readiness'\''\)' "$PROTOCOL" \
  'frontend input readiness parser must read the daemon input_readiness object'
require 'objectField\(result, '\''terminal_receipt'\''\)' "$PROTOCOL" \
  'frontend terminal receipt parser must read the daemon terminal_receipt object'
require 'interactiveReady: value\.interactive_ready === true' "$PROTOCOL" \
  'frontend input readiness must expose daemon interactive_ready'
require 'blockedReason: stringField\(value, '\''blocked_reason'\''\)' "$PROTOCOL" \
  'frontend input readiness must expose daemon blocked_reason'
require 'remoteDesktopSessionTerminal' "$PROTOCOL" \
  'frontend must expose one helper for terminal remote desktop session state'
require 'remoteDesktopInputFrameAllowed' "$PROTOCOL" \
  'frontend must derive remote desktop input eligibility from runtime target tracking and input policy'
require 'remoteDesktopInputFrameAllowed' "$STORE" \
  'frontend store input sender must use the remote desktop input eligibility helper'
require 'remoteDesktopSessionTerminal\(session\)' "$STORE" \
  'frontend store must use terminal session state before ending/reusing RemoteApp sessions'
require 'guardTerminalRemoteDesktopSessionPatch' "$STORE" \
  'frontend store must centralize stale async protection for terminal RemoteApp sessions'
require 'remoteDesktopSessionTerminal\(current\)' "$STORE" \
  'frontend store terminal guard must inspect the current RemoteApp session state'
require '!remoteDesktopSessionTerminal\(next\)' "$STORE" \
  'frontend store terminal guard must reject stale non-terminal projections over terminal sessions'
require 'session: view \? \{ \.\.\.view, sessionToken: undefined \} : null' "$STORE" \
  'frontend end_session must preserve daemon terminal session view while clearing the session token'
require 'view\.inputReadiness\.interactiveReady !== true' "$PROTOCOL" \
  'frontend input gating must fail closed when daemon input_readiness says interactive is not ready'
require 'view\.inputReadiness\.pointerEnabled === true' "$PROTOCOL" \
  'frontend pointer input gating must use daemon input_readiness pointer readiness when present'
require 'view\.inputReadiness\.keyboardEnabled === true' "$PROTOCOL" \
  'frontend keyboard input gating must use daemon input_readiness keyboard readiness when present'
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
require 'input_control: true' "$STORE_TEST" \
  'frontend store tests must prove interactive RemoteApp creation requests input_control consent'
require 'keeps remote desktop consent and session input policy view-only when interactive is disabled' "$STORE_TEST" \
  'frontend store tests must prove view-only RemoteApp creation does not request input-control authority'
require 'input_control: false' "$STORE_TEST" \
  'frontend store tests must prove disabled interactive mode keeps consent input_control=false'
require "mode: 'view_only'" "$STORE_TEST" \
  'frontend store tests must prove disabled interactive mode creates a view-only session'
require 'surfaces remote desktop recovery events from the session watcher' "$STORE_TEST" \
  'frontend store tests must prove watch_events recovery events affect UI/transport state'
require 'target_permission_revoked' "$STORE_TEST" \
  'frontend store tests must prove permission-revoked events synchronize daemon terminal receipts'
require 'session\?\.sessionToken\)\.toBeUndefined\(\)' "$STORE_TEST" \
  'frontend store tests must prove synchronized terminal sessions clear bearer tokens'
require 'projects target diagnostics and tracking state from the runtime session view' "$PROTOCOL_TEST" \
  'frontend protocol tests must cover latest target diagnostic and tracking projection'
require 'does not treat ordinary view-only input state as target recovery failure' "$PROTOCOL_TEST" \
  'frontend protocol tests must prove view-only input state is not misreported as target loss'
require 'gates frontend input frames on runtime target tracking and input policy' "$PROTOCOL_TEST" \
  'frontend protocol tests must cover target-tracking and input-policy input gating'
require 'prefers runtime input readiness over legacy input policy for input gating' "$PROTOCOL_TEST" \
  'frontend protocol tests must prove daemon input_readiness overrides legacy input policy'
require 'blocked\.inputReadiness' "$PROTOCOL_TEST" \
  'frontend protocol tests must assert parsed blocked input readiness'
require 'interactiveReady: false' "$PROTOCOL_TEST" \
  'frontend protocol tests must cover non-ready interactive input readiness'
require 'keeps base media controls available when remote desktop target refresh fails' "$ACCESS_TEST" \
  'frontend access tests must prove remote target failure does not disable base media'
require 'runs the remote desktop UI flow from target picker through session end' "$ACCESS_TEST" \
  'frontend access tests must prove picker-to-session-to-end remote desktop UI flow'
require 'surfaces daemon remote desktop input readiness in session details' "$ACCESS_TEST" \
  'frontend access tests must prove daemon input_readiness appears in session details'
require 'input_injection_unavailable' "$ACCESS_TEST" \
  'frontend access tests must cover visible blocked input readiness reason'
require 'interactive->view_only' "$ACCESS_TEST" \
  'frontend access tests must cover visible interactive-to-view-only input downgrade'
require 'surfaces daemon remote desktop terminal receipts in session details' "$ACCESS_TEST" \
  'frontend access tests must prove daemon terminal_receipt appears in session details'
require 'terminal caller_ended #9' "$ACCESS_TEST" \
  'frontend access tests must cover visible RemoteApp terminal receipt reason and event sequence'
require 'remote_desktop\.grant_consent' "$ACCESS_TEST" \
  'frontend access tests must prove UI flow grants target-scoped consent'
require 'input_control: true' "$ACCESS_TEST" \
  'frontend access tests must prove UI flow requests input_control consent for default interactive RemoteApp sessions'
require 'remote_desktop\.create_session' "$ACCESS_TEST" \
  'frontend access tests must prove UI flow creates a remote desktop session'
require 'remote_desktop\.watch_events' "$ACCESS_TEST" \
  'frontend access tests must prove UI flow starts the watch_events stream'
require 'remote_desktop\.end_session' "$ACCESS_TEST" \
  'frontend access tests must prove UI flow ends the session'
require 'subject_ura: screenResource\.resource_ura' "$ACCESS_TEST" \
  'frontend access tests must assert selected target subject propagation through UI flow'
require 'terminal \{session\.terminalReceipt\.reasonCode' "$ACCESS" \
  'frontend session details must render daemon terminal_receipt reason'
require 'terminalEventSequence' "$ACCESS" \
  'frontend session details must render daemon terminal receipt event sequence'
require 'projects remote desktop terminal receipts from daemon session views' "$PROTOCOL_TEST" \
  'frontend protocol tests must cover daemon terminal_receipt projection'
require 'remoteDesktopSessionTerminal\(view\)' "$PROTOCOL_TEST" \
  'frontend protocol tests must prove terminal receipt marks a terminal RemoteApp session'

printf 'check-remoteapp-frontend-invocation-boundary: ok\n'
