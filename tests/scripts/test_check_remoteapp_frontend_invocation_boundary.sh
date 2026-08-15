#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-frontend-invocation-boundary.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

FRONTEND_SRC="$SANDBOX/Frontend/src"
mkdir -p \
  "$FRONTEND_SRC/store" \
  "$FRONTEND_SRC/components/easynet" \
  "$FRONTEND_SRC/pages/easynet" \
  "$FRONTEND_SRC/lib/api"

cat >"$FRONTEND_SRC/store/media-channel-invocation.ts" <<'TS'
const REMOTE_DESKTOP_SESSION_SUBJECT_REQUIRED_ABILITIES = new Set([
  'remote_desktop.grant_consent',
  'remote_desktop.create_session',
  'remote_desktop.attach',
  'remote_desktop.set_description',
  'remote_desktop.add_ice_candidate',
  'remote_desktop.report_client_state',
  'remote_desktop.show_session',
  'remote_desktop.refresh_lease',
  'remote_desktop.watch_events',
  'remote_desktop.end_session',
])

export async function invokeMediaUnaryResponse(ability: string, opts: { subjectURA?: string; args: Record<string, unknown> }) {
  requireRemoteDesktopSessionSubject(ability, opts.subjectURA)
  return issueAuthenticatedBrowserRootInvocation({
    ability,
    subject: opts.subjectURA
      ? { kind: 'ura', ura: opts.subjectURA }
      : { kind: 'authenticated-user' },
    arguments: opts.args,
  })
}

export async function invokeMediaStream(ability: string, opts: { subjectURA?: string; args: Record<string, unknown> }) {
  requireRemoteDesktopSessionSubject(ability, opts.subjectURA)
  return issueAuthenticatedBrowserRootInvocation({
    ability,
    subject: opts.subjectURA
      ? { kind: 'ura', ura: opts.subjectURA }
      : { kind: 'authenticated-user' },
    arguments: opts.args,
  })
}

function requireRemoteDesktopSessionSubject(ability: string, subjectURA: string | undefined): void {
  if (!REMOTE_DESKTOP_SESSION_SUBJECT_REQUIRED_ABILITIES.has(ability)) return
  if (subjectURA?.trim()) return
  throw new Error(`${ability}: remote desktop session subject_ura is required`)
}
TS

cat >"$FRONTEND_SRC/store/media-channel-store.ts" <<'TS'
import { remoteDesktopInputFrameAllowed } from '@/lib/api/remote-desktop-protocol'

export async function rdCreate(entry: Entry, env: { resource?: { resource_ura: string } }) {
  const resource = env.resource
  if (!resource) return
  const consent = await invokeMediaUnaryResponse('remote_desktop.grant_consent', {
    subjectURA: resource.resource_ura,
    args: { intent: 'remote_desktop_session' },
  })
  const causalContext = remoteDesktopConsentCausalContext(consent)
  const consentTicket = remoteDesktopConsentTicket(consent)
  const result = await invokeMediaUnary('remote_desktop.create_session', {
    subjectURA: resource.resource_ura,
    causalContext,
    args: {
      consent_ticket: consentTicket,
      mode: 'view_only',
    },
  })
  assertRemoteDesktopCreateSessionIdentity(result)
  const view = projectRemoteDesktopView(result)
  return view
}

export const actions = {
  rdReportClientMediaState: (key: string, state: 'presenting' | 'stalled' | 'detached') => reportClientMediaState(key, state),
  rdSendInput: (key, frame) => {
    const session = entries[key]?.session
    if (!session || !remoteDesktopInputFrameAllowed(session, frame)) return false
    const channel = refsFor(key).inputChannel
    if (!channel || channel.readyState !== 'open') return false
    channel.send(JSON.stringify(frame))
    return true
  },
  rdRequestPermission: async (key: string) => {
    const entry = entries[key]
    const result = await invokeMediaUnary('remote_desktop.request_permission', {
      deviceUra: entry.deviceUra,
      args: {},
    })
    return result
  },
}

function reportClientMediaState(key: string, state: 'presenting' | 'stalled' | 'detached') {
  const currentView = entries[key].session
  const epoch = currentView.transportEpoch
  const desired = state
  return invokeMediaUnary('remote_desktop.report_client_state', {
    deviceUra: entries[key].deviceUra,
    subjectURA: currentView.subjectUra,
    causalContext: remoteDesktopSessionCausalContext(currentView),
    args: {
      session_id: currentView.sessionId,
      session_token: currentView.sessionToken,
      transport_epoch: epoch,
      state: desired,
    },
  })
}

function presentationTimeoutGuard(key: string, currentView: RemoteDesktopView) {
  const currentRefs = refsFor(key)
  if (
    currentRefs.clientMediaReportedState === 'presenting' ||
    currentView?.clientMediaReady === true
  ) return
  patchEntry(key, { webrtcStatus: 'remote desktop did not present a frame within 10s' })
}

function onConnectionStateChange(pc: RTCPeerConnection) {
  if (pc.connectionState === 'connected') updateWebRtcStatus()
}

function assertRemoteDesktopCreateSessionIdentity(result: Record<string, unknown> | undefined): void {
  if (!stringField(result, 'session_id')) {
    throw new Error('remote_desktop.create_session response did not include session_id')
  }
  if (!stringField(result, 'subject_ura')) {
    throw new Error('remote_desktop.create_session response did not include subject_ura')
  }
}
TS

cat >"$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx" <<'TSX'
function WebRtcVideoViewport({
  stream,
  onPresented,
  onStalled,
}: {
  stream: MediaStream
  onPresented: () => void
  onStalled: () => void
}) {
  const videoWithFrameCallback = video as HTMLVideoElement & {
    requestVideoFrameCallback?: (callback: () => void) => number
  }
  videoWithFrameCallback.requestVideoFrameCallback?.(() => {
    onPresented()
  })
  const handlePlaying = () => {
    if (!videoWithFrameCallback.requestVideoFrameCallback) onPresented()
  }
  video.addEventListener('playing', handlePlaying)
  video.addEventListener('stalled', onStalled)
  return <video />
}

export function DeviceMediaAccess() {
  const baseRuntimeReady = online === true && !resourceRuntimeOffline
  const remoteTargetReady = baseRuntimeReady && !remoteTargetError
  const reportClientMediaState = useMediaChannelStore((state) => state.rdReportClientMediaState)
  const reportPresented = useCallback(
    () => reportClientMediaState(channelKey, 'presenting'),
    [channelKey, reportClientMediaState],
  )
  const reportStalled = useCallback(
    () => reportClientMediaState(channelKey, 'stalled'),
    [channelKey, reportClientMediaState],
  )
  const remoteTargetData = listRemoteDesktopTargets()
  const result = invokeMediaUnary('resource.refresh_remote_targets', {})
  const screenResources = remoteTargetData.resources
  const screenResource = screenResources.find((resource) => resource.resource_ura === selectedScreenURA)
  if (selectedScreenURA && !screenResources.some((resource) => resource.resource_ura === selectedScreenURA)) {
    setSelectedScreenURA(undefined)
  }
  const viewport = (
    <WebRtcVideoViewport
      stream={preview.mediaStream}
      onPresented={reportPresented}
      onStalled={reportStalled}
    />
  )
  return remoteTargetReady && screenResource ? <div /> : null
}
TSX

cat >"$FRONTEND_SRC/pages/easynet/DeviceMediaWorkspacePage.tsx" <<'TSX'
export function DeviceMediaWorkspacePage() {
  const remoteTargetData = listRemoteDesktopTargets()
  const query = {
    refetchInterval: runtimeOnline ? 5000 : false,
  }
  const screenResources = remoteTargetData.resources
  const screenResource = screenResources.find((resource) => resource.resource_ura === entry.session?.subjectUra)
  if (!screenResource) {
    return <div>Session target is no longer advertised by the live target inventory</div>
  }
  return <MediaChannelPanel screenResource={screenResource} />
}
TSX

cat >"$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts" <<'TS'
const productionGate = objectField(result, 'production_gate')
const productionReadiness = remoteDesktopProductionReadinessFromResult(result)

export function remoteDesktopViewFromResult(result: Record<string, unknown> | undefined) {
  return {
    productionReady: result?.production_media_ready === true || productionReadiness?.ready === true,
    productionReadiness,
    productionBlockedReason: productionReadiness?.blockedReason ?? stringField(productionGate, 'reason'),
    latestTargetDiagnostic: remoteDesktopTargetDiagnosticFromValue(objectField(result, 'latest_target_diagnostic')),
    targetTracking: remoteDesktopTargetTrackingFromValue(objectField(result, 'target_tracking')),
  }
}

function remoteDesktopTargetDiagnosticFromValue(value: Record<string, unknown> | undefined) {
  return {
    frontendAction: stringField(value, 'frontend_action'),
  }
}

function remoteDesktopTargetTrackingFromValue(value: Record<string, unknown> | undefined) {
  return {
    inputEnabled: value?.input_enabled === true,
  }
}

export function remoteDesktopTargetRecoveryMessage(view: RemoteDesktopView): string | undefined {
  return view.latestTargetDiagnostic?.frontendAction
}

export function remoteDesktopInputFrameAllowed(view: RemoteDesktopView, frame: Record<string, unknown>): boolean {
  return view.targetTracking?.inputEnabled !== false && frame.type !== 'blocked'
}

export function remoteDesktopProductionBlockedMessage(view: RemoteDesktopView): string {
  const reason = remoteDesktopTargetRecoveryMessage(view)
    ?? view.productionBlockedReason
    ?? 'native_media_plugin_required'
  return reason
}
TS

cat >"$FRONTEND_SRC/lib/api/remote-desktop-protocol.test.ts" <<'TS'
it('projects target diagnostics and tracking state from the runtime session view', () => {
  expect(remoteDesktopViewFromResult({
    latest_target_diagnostic: { frontend_action: 'refresh_targets' },
    target_tracking: { input_enabled: false },
  }).latestTargetDiagnostic.frontendAction).toBe('refresh_targets')
})

it('does not treat ordinary view-only input state as target recovery failure', () => {
  expect(remoteDesktopTargetRecoveryMessage({
    targetTracking: { inputEnabled: false },
  })).toBeUndefined()
})

it('gates frontend input frames on runtime target tracking and input policy', () => {
  expect(remoteDesktopInputFrameAllowed({
    targetTracking: { inputEnabled: false },
  }, { type: 'pointer' })).toBe(false)
})
TS

cat >"$FRONTEND_SRC/store/media-channel-invocation.test.ts" <<'TS'
it('passes selected subject into create session envelope', async () => {
  await invokeMediaUnary('remote_desktop.create_session', {
    subjectURA: 'easynet:///r/test/resource/device.mac-1/streams/display-1',
    args: {},
  })
})

it.each([
  'remote_desktop.grant_consent',
  'remote_desktop.create_session',
  'remote_desktop.attach',
  'remote_desktop.report_client_state',
])('requires an explicit remote desktop subject for %s', async (ability) => {
  await expect(invokeMediaUnary(ability, { args: {} }))
    .rejects.toThrow(`${ability}: remote desktop session subject_ura is required`)
})
TS

cat >"$FRONTEND_SRC/store/media-channel-store.test.ts" <<'TS'
it('reports missing remote desktop session subject before projection fallback', async () => {
  expect(entry.error).toContain('remote_desktop.create_session response did not include subject_ura')
})
TS

cat >"$FRONTEND_SRC/components/easynet/DeviceMediaAccess.test.tsx" <<'TSX'
it('keeps base media controls available when remote desktop target refresh fails', async () => {
  expect(screen.getByRole('button', { name: /Camera/i })).not.toBeDisabled()
  expect(screen.getByRole('button', { name: /Remote desktop/i })).toBeDisabled()
})
TSX

CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

perl -0pi -e "s/  'remote_desktop\\.create_session',\\n//" \
  "$FRONTEND_SRC/store/media-channel-invocation.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted missing create_session subject requirement" >&2
  exit 1
fi
perl -0pi -e "s/  'remote_desktop\\.grant_consent',/  'remote_desktop.grant_consent',\\n  'remote_desktop.create_session',/" \
  "$FRONTEND_SRC/store/media-channel-invocation.ts"

perl -0pi -e "s/  'remote_desktop\\.end_session',/  'remote_desktop.end_session',\\n  'remote_desktop.request_permission',/" \
  "$FRONTEND_SRC/store/media-channel-invocation.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted target-subject requirement for request_permission" >&2
  exit 1
fi
perl -0pi -e "s/\\n  'remote_desktop\\.request_permission',//" \
  "$FRONTEND_SRC/store/media-channel-invocation.ts"

perl -0pi -e "s/mode: 'view_only',/mode: 'view_only',\\n      subject_ura: resource.resource_ura,/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted create_session args.subject_ura" >&2
  exit 1
fi
perl -0pi -e "s/\\n      subject_ura: resource\\.resource_ura,//" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e 's/(deviceUra: entry\.deviceUra,\n)/$1      subjectURA: selectedTarget.resource_ura,\n/' \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted target-scoped request_permission" >&2
  exit 1
fi
perl -0pi -e "s/\\n      subjectURA: selectedTarget\\.resource_ura,//" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e 's/screenResources\.find\(\(resource\) => resource\.resource_ura === selectedScreenURA\)/screenResources[0]/' \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted first-target access fallback" >&2
  exit 1
fi
perl -0pi -e 's/screenResources\[0\]/screenResources.find((resource) => resource.resource_ura === selectedScreenURA)/' \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx"

perl -0pi -e 's/screenResources\.find\(\(resource\) => resource\.resource_ura === entry\.session\?\.subjectUra\)/screenResources[0]/' \
  "$FRONTEND_SRC/pages/easynet/DeviceMediaWorkspacePage.tsx"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted first-target workspace fallback" >&2
  exit 1
fi
perl -0pi -e 's/screenResources\[0\]/screenResources.find((resource) => resource.resource_ura === entry.session?.subjectUra)/' \
  "$FRONTEND_SRC/pages/easynet/DeviceMediaWorkspacePage.tsx"

perl -0pi -e 's/requestVideoFrameCallback/requestPeerConnectionCallback/g' \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted missing decoded-frame callback presentation gate" >&2
  exit 1
fi
perl -0pi -e 's/requestPeerConnectionCallback/requestVideoFrameCallback/g' \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx"

perl -0pi -e "s/reportClientMediaState\\(channelKey, 'presenting'\\)/reportClientMediaState(channelKey, 'stalled')/" \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted decoded-frame callback that does not report presenting" >&2
  exit 1
fi
perl -0pi -e "s/reportClientMediaState\\(channelKey, 'stalled'\\)/reportClientMediaState(channelKey, 'presenting')/" \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx"

perl -0pi -e "s/if \\(pc\\.connectionState === 'connected'\\) updateWebRtcStatus\\(\\)/if (pc.connectionState === 'connected') reportClientMediaState(key, 'presenting')/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted peer-connected client presentation report" >&2
  exit 1
fi
perl -0pi -e "s/if \\(pc\\.connectionState === 'connected'\\) reportClientMediaState\\(key, 'presenting'\\)/if (pc.connectionState === 'connected') updateWebRtcStatus()/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e 's/remoteDesktopInputFrameAllowed\(session, frame\)/true/' \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted input send without runtime target-tracking/policy gate" >&2
  exit 1
fi
perl -0pi -e 's/if \(!session \|\| true\) return false/if (!session || remoteDesktopInputFrameAllowed(session, frame)) return false/' \
  "$FRONTEND_SRC/store/media-channel-store.ts"
perl -0pi -e 's/if \(!session \|\| remoteDesktopInputFrameAllowed\(session, frame\)\) return false/if (!session || !remoteDesktopInputFrameAllowed(session, frame)) return false/' \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e "s/    latestTargetDiagnostic: remoteDesktopTargetDiagnosticFromValue\\(objectField\\(result, 'latest_target_diagnostic'\\)\\),\\n    targetTracking: remoteDesktopTargetTrackingFromValue\\(objectField\\(result, 'target_tracking'\\)\\),//" \
  "$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted missing target diagnostic/tracking projection" >&2
  exit 1
fi
perl -0pi -e "s/    productionBlockedReason: productionReadiness\\?\\.blockedReason \\?\\? stringField\\(productionGate, 'reason'\\),/    productionBlockedReason: productionReadiness?.blockedReason ?? stringField(productionGate, 'reason'),\\n    latestTargetDiagnostic: remoteDesktopTargetDiagnosticFromValue(objectField(result, 'latest_target_diagnostic')),\\n    targetTracking: remoteDesktopTargetTrackingFromValue(objectField(result, 'target_tracking')),/" \
  "$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"

perl -0pi -e 's/const reason = remoteDesktopTargetRecoveryMessage\(view\)\n    \?\? view\.productionBlockedReason/const reason = view.productionBlockedReason/' \
  "$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted production blocked message without target recovery priority" >&2
  exit 1
fi
perl -0pi -e 's/const reason = view\.productionBlockedReason/const reason = remoteDesktopTargetRecoveryMessage(view)\n    ?? view.productionBlockedReason/' \
  "$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"

perl -0pi -e 's/productionReady: result\?\.production_media_ready === true \|\| productionReadiness\?\.ready === true/productionReady: productionGate?.ready === true || mediaBackends.some(isRemoteDesktopProductionBackend)/' \
  "$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted production readiness derived from capability gates" >&2
  exit 1
fi

echo "test_check_remoteapp_frontend_invocation_boundary.sh: all cases passed"
