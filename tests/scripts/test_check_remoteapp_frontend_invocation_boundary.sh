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
import { remoteDesktopInputFrameAllowed, remoteDesktopSessionTerminal } from '@/lib/api/remote-desktop-protocol'

const REMOTE_DESKTOP_INPUT_MAX_BUFFERED_BYTES = 64 * 1024

function guardTerminalRemoteDesktopSessionPatch(prev: Entry, patch: Partial<Entry>): Partial<Entry> {
  const current = prev.session
  const next = patch.session
  if (current && next && remoteDesktopSessionTerminal(current) && !remoteDesktopSessionTerminal(next)) {
    return { ...patch, session: current, attached: false }
  }
  return patch
}

type RemoteDesktopSessionInputIntent = {
  inputControlRequested: boolean
  mode: 'interactive' | 'view_only'
  inputPolicy: {
    keyboard_enabled: boolean
    pointer_enabled: boolean
    clipboard_enabled: false
    file_drop_enabled: false
  }
}

function remoteDesktopSessionInputIntent(entry: Entry): RemoteDesktopSessionInputIntent {
  const inputControlRequested = entry.interactive === true
  return {
    inputControlRequested,
    mode: inputControlRequested ? 'interactive' : 'view_only',
    inputPolicy: {
      keyboard_enabled: inputControlRequested,
      pointer_enabled: inputControlRequested,
      clipboard_enabled: false,
      file_drop_enabled: false,
    },
  }
}

export async function rdCreate(entry: Entry, env: { resource?: { resource_ura: string } }) {
  const resource = env.resource
  if (!resource) return
  const sessionInputIntent = remoteDesktopSessionInputIntent(entry)
  const consent = await invokeMediaUnaryResponse('remote_desktop.grant_consent', {
    subjectURA: resource.resource_ura,
    args: {
      intent: 'remote_desktop_session',
      input_control: sessionInputIntent.inputControlRequested,
    },
  })
  const causalContext = remoteDesktopConsentCausalContext(consent)
  const consentTicket = remoteDesktopConsentTicket(consent)
  const result = await invokeMediaUnary('remote_desktop.create_session', {
    subjectURA: resource.resource_ura,
    causalContext,
    args: {
      consent_ticket: consentTicket,
      mode: sessionInputIntent.mode,
      input_policy: sessionInputIntent.inputPolicy,
    },
  })
  assertRemoteDesktopCreateSessionIdentity(result)
  const view = projectRemoteDesktopView(result)
  const negotiated = view
  startRemoteDesktopEventWatch(key, negotiated)
  return view
}

function stopRemoteDesktopEventWatch(key: string) {
  refsFor(key).remoteDesktopEventsAbort?.abort()
}

function startRemoteDesktopEventWatch(key: string, view: RemoteDesktopView) {
  const causalContext = remoteDesktopSessionCausalContext(view)
  return invokeMediaStream(
    'remote_desktop.watch_events',
    {
      deviceUra: entries[key].deviceUra,
      subjectURA: view.subjectUra,
      causalContext,
      args: { session_id: view.sessionId, session_token: view.sessionToken },
      timeoutMs: 0,
    },
  )
}

function applyRemoteDesktopSessionEventEffect(key: string, sessionId: string, event: RemoteDesktopEvent) {
  const recovery = remoteDesktopSessionEventRecovery(event)
  if (!recovery) return
  if (recovery.closeLocalTransport) stopRemoteDesktopEventWatch(key)
  if (recovery.syncTerminalSession) syncRemoteDesktopTerminalSession(key, sessionId)
  patchEntry(key, {
    attached: false,
    webrtcStatus: recovery.status,
  })
}

async function syncRemoteDesktopTerminalSession(key: string, sessionId: string) {
  try {
    const result = await invokeMediaUnary('remote_desktop.show_session', {
      deviceUra: entries[key].deviceUra,
      subjectURA: entries[key].session.subjectUra,
      causalContext: remoteDesktopSessionCausalContext(entries[key].session),
      args: { session_id: sessionId, session_token: entries[key].session.sessionToken },
    })
    patchEntry(key, { session: projectRemoteDesktopView(result) })
  } catch {
    patchEntry(key, { webrtcStatus: 'terminal sync failed' })
  }
}

function remoteDesktopSessionEventRecovery(event: RemoteDesktopEvent) {
  if (event.eventType === 'TARGET_PERMISSION_REVOKED') {
    return {
      status: 'remote desktop permission was revoked',
      closeLocalTransport: true,
      syncTerminalSession: true,
    }
  }
  if (event.eventType === 'INPUT_FRAME_REJECTED') {
    return {
      status: 'remote desktop input rejected (stale_pointer_target_geometry)',
      closeLocalTransport: false,
    }
  }
  if (event.eventType === 'INPUT_CHANNEL_OPENED') {
    return {
      status: 'remote desktop input blocked (input_injection_unavailable)',
      closeLocalTransport: false,
    }
  }
  if (event.eventType === 'SESSION_DEGRADED') {
    return {
      status: 'remote desktop session needs retry',
      closeLocalTransport: false,
    }
  }
  return null
}

function closeAttach(key: string, reason: string, options?: { keepSessionPolling?: boolean }) {
  refsFor(key).remoteDesktopEventsAbort?.abort()
}

function patchPreview(key: string, patch: Record<string, unknown>) {
  return patch
}

const suspendEntryForOffline = (key: string, reason: string) => {
  const entry = entries[key]
  if (!entry) return
  if (entry.channel === 'remoteDesktop') {
    patchEntry(key, {
      error: reason,
      attached: false,
      loading: false,
      webrtcStatus: 'device offline; remote desktop session preserved for reconnect',
    })
  }
}

const resumeEntryFromOffline = (key: string) => {
  const entry = entries[key]
  if (!entry) return
  if (entry.channel === 'remoteDesktop') {
    const session = entry.session
    if (!session || remoteDesktopSessionTerminal(session)) return
    void resumeRemoteDesktopSessionAfterOffline(key, session)
  }
}

const resumeRemoteDesktopSessionAfterOffline = async (key: string, session: RemoteDesktopView) => {
  const refs = refsFor(key)
  refs.remoteDesktopResumeIdentity = `${session.sessionId}:${session.sessionToken}`
  const result = await invokeMediaUnary('remote_desktop.show_session', {
    deviceUra: entries[key].deviceUra,
    subjectURA: session.subjectUra,
    causalContext: remoteDesktopSessionCausalContext(session),
    args: { session_id: session.sessionId, session_token: session.sessionToken },
  })
  const view = projectRemoteDesktopView(result, session.sessionToken)
  const negotiated = await startWebRtc(key, view, { endSessionOnTransportFailure: false })
  patchEntry(key, { session: negotiated, webrtcStatus: 'remote desktop transport reconnected' })
}

async function startWebRtc(
  key: string,
  view: RemoteDesktopView,
  options: { endSessionOnTransportFailure?: boolean } = {},
) {
  const endSessionOnTransportFailure = options.endSessionOnTransportFailure ?? true
  if (!endSessionOnTransportFailure) {
    patchEntry(key, { webrtcStatus: 'remote desktop transport failed; session preserved for reconnect' })
    closeAttach(key, 'webrtc_failed_resume', { keepSessionPolling: true })
  }
  return view
}

export const actions = {
  rdReportClientMediaState: (key: string, state: 'presenting' | 'stalled' | 'detached') => reportClientMediaState(key, state),
  rdSendInput: (key, frame) => {
    const session = entries[key]?.session
    if (!session || !remoteDesktopInputFrameAllowed(session, frame)) return false
    const refs = refsFor(key)
    const channel = refs.inputChannel
    if (!channel || channel.readyState !== 'open') return false
    if (channel.bufferedAmount > REMOTE_DESKTOP_INPUT_MAX_BUFFERED_BYTES) {
      patchEntry(key, {
        webrtcStatus: `input backpressure: ${channel.bufferedAmount} bytes buffered; dropping stale RemoteApp input`,
      })
      return false
    }
    refs.remoteDesktopInputSequence = refs.remoteDesktopInputSequence >= Number.MAX_SAFE_INTEGER - 1
      ? 1
      : refs.remoteDesktopInputSequence + 1
    channel.send(JSON.stringify({
      ...frame,
      client_sequence: refs.remoteDesktopInputSequence,
      sent_at_ms: Date.now(),
    }))
    return true
  },
  rdRequestPermission: async (key: string) => {
    const entry = entries[key]
    const result = await invokeMediaUnary('remote_desktop.request_permission', {
      deviceUra: entry.deviceUra,
      args: {},
    })
    return remoteDesktopPermissionRequestResult(result)
  },
  rdCheckPermission: async (key: string) => {
    const entry = entries[key]
    const result = await invokeMediaUnary('remote_desktop.permission_status', {
      deviceUra: entry.deviceUra,
      args: {},
    })
    return remoteDesktopPermissionStatusResult(result)
  },
  rdEnd: async (key: string) => {
    const session = entries[key]?.session
    if (!session) return
    if (remoteDesktopSessionTerminal(session)) return
    const result = await invokeRemoteDesktopEndSessionWithRetry()
    const view = projectRemoteDesktopView(result, session.sessionToken)
    patchEntry(key, {
      session: view ? { ...view, sessionToken: undefined } : null,
    })
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

function remoteDesktopPermissionRequestResult(result: Record<string, unknown> | undefined) {
  const inputPermission = objectField(result, 'input_permission')
  return {
    message: inputPermission
      ? 'Accessibility input permission requested but still unavailable for pointer/keyboard control.'
      : 'Screen Recording permission is still unavailable.',
  }
}

function remoteDesktopPermissionStatusResult(result: Record<string, unknown> | undefined) {
  const inputPermission = objectField(result, 'input_permission')
  return {
    message: inputPermission
      ? 'Accessibility input permission is not granted for pointer/keyboard control.'
      : 'Screen Recording permission is not granted.',
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
  const targetGeometryRevision = entry.session?.targetTracking?.targetGeometryRevision
  const pointerFrame = {
    type: 'pointer',
    target_geometry_revision: targetGeometryRevision,
  }
  const remoteTargetData = listRemoteDesktopTargets()
  const result = invokeMediaUnary('resource.refresh_remote_targets', {})
  const screenResources = remoteTargetData.resources
  const screenResource = screenResources.find((resource) => resource.resource_ura === selectedScreenURA)
  const session = entry.session
  const inputReadinessDetails = session ? remoteDesktopInputReadinessLabel(session) : undefined
  const terminalReceiptDetails = session?.terminalReceipt
    ? `terminal {session.terminalReceipt.reasonCode ?? session.terminalReceipt.receiptType} #${session.terminalReceipt.terminalEventSequence}`
    : undefined
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
  return remoteTargetReady && screenResource ? <div>{inputReadinessDetails}{terminalReceiptDetails}</div> : null
}

function remoteDesktopInputReadinessLabel(view: RemoteDesktopView) {
  const readiness = view.inputReadiness
  if (!readiness) return `input ${view.inputPolicy}`
  const effectiveMode = readiness.effectiveMode ?? (readiness.interactiveReady ? 'interactive' : 'view_only')
  const requestedMode = readiness.requestedMode && readiness.requestedMode !== effectiveMode
    ? `${readiness.requestedMode}->${effectiveMode}`
    : effectiveMode
  const state = readiness.interactiveReady ? `${requestedMode} ready` : requestedMode
  return readiness.blockedReason ? `input ${state} · ${readiness.blockedReason}` : `input ${state}`
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
const inputReadiness = remoteDesktopInputReadinessFromResult(result)

export type RemoteDesktopInputReadiness = {
  requestedMode?: string
  effectiveMode?: string
  interactiveReady: boolean
  blockedReason?: string
  inputScope?: string
  pointerEnabled: boolean
  keyboardEnabled: boolean
}

export type RemoteDesktopTerminalReceipt = {
  receiptType: string
  sessionId: string
  reasonCode?: string
  terminalEventSequence?: number
}

type RemoteDesktopView = {
  productionBlockedReason?: string
  latestTargetDiagnostic?: { frontendAction?: string }
  targetTracking?: { inputEnabled?: boolean }
  inputPolicy: string
  inputReadiness?: RemoteDesktopInputReadiness
  terminalReceipt?: RemoteDesktopTerminalReceipt
  state?: string
  sessionId?: string
}

export function remoteDesktopViewFromResult(result: Record<string, unknown> | undefined) {
  return {
    productionReady: productionReadiness?.ready === true,
    productionReadiness,
    productionBlockedReason: productionReadiness?.blockedReason ?? stringField(productionGate, 'reason'),
    latestTargetDiagnostic: remoteDesktopTargetDiagnosticFromValue(objectField(result, 'latest_target_diagnostic')),
    targetTracking: remoteDesktopTargetTrackingFromValue(objectField(result, 'target_tracking')),
    inputPolicy: 'view-only',
    inputReadiness,
    terminalReceipt: remoteDesktopTerminalReceiptFromResult(result),
  }
}

function remoteDesktopInputReadinessFromResult(
  result: Record<string, unknown> | undefined,
): RemoteDesktopInputReadiness | undefined {
  const value = objectField(result, 'input_readiness')
  if (!value) return undefined
  return {
    requestedMode: stringField(value, 'requested_mode'),
    effectiveMode: stringField(value, 'effective_mode'),
    interactiveReady: value.interactive_ready === true,
    blockedReason: stringField(value, 'blocked_reason'),
    inputScope: stringField(value, 'input_scope'),
    pointerEnabled: value.pointer_enabled === true,
    keyboardEnabled: value.keyboard_enabled === true,
  }
}

function remoteDesktopTerminalReceiptFromResult(
  result: Record<string, unknown> | undefined,
): RemoteDesktopTerminalReceipt | undefined {
  const value = objectField(result, 'terminal_receipt')
  if (!value) return undefined
  return {
    receiptType: stringField(value, 'receipt_type'),
    sessionId: stringField(value, 'session_id'),
    reasonCode: stringField(value, 'reason_code'),
    terminalEventSequence: numberField(value, 'terminal_event_sequence'),
  }
}

export function remoteDesktopSessionTerminal(view: RemoteDesktopView | null | undefined): boolean {
  return view?.state === 'closed' || view?.terminalReceipt?.sessionId === view?.sessionId
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
  if (view.targetTracking?.inputEnabled === false) return false
  const frameType = typeof frame.type === 'string' ? frame.type : undefined
  if (view.inputReadiness) {
    if (view.inputReadiness.interactiveReady !== true) return false
    if (frameType === 'pointer' || frameType === 'wheel') return view.inputReadiness.pointerEnabled === true
    if (frameType === 'key' || frameType === 'keyboard') return view.inputReadiness.keyboardEnabled === true
    return false
  }
  return frame.type !== 'blocked'
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

it('prefers runtime input readiness over legacy input policy for input gating', () => {
  const blocked = {
    targetTracking: { inputEnabled: true },
    inputPolicy: 'keyboard+pointer',
    inputReadiness: {
      interactiveReady: false,
      blockedReason: 'target_scoped_keyboard_pointer_dispatch_unsafe',
      pointerEnabled: false,
      keyboardEnabled: false,
    },
  }
  expect(blocked.inputReadiness).toBeDefined()
  expect(blocked.inputReadiness).toMatchObject({ interactiveReady: false })
  expect(remoteDesktopInputFrameAllowed(blocked, { type: 'pointer' })).toBe(false)
})

it('projects remote desktop terminal receipts from daemon session views', () => {
  const view = remoteDesktopViewFromResult({
    session_id: 'rd-1',
    state: 'closed',
    terminal_receipt: {
      receipt_type: 'remoteapp.session.terminal.v1',
      session_id: 'rd-1',
      reason_code: 'caller_ended',
      terminal_event_sequence: 9,
    },
  })
  expect(view.terminalReceipt).toMatchObject({ sessionId: 'rd-1' })
  expect(remoteDesktopSessionTerminal(view)).toBe(true)
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

it('runs remote desktop WebRTC sessions with a target-scoped event watcher', async () => {
  expect(mocks.invokeMediaUnaryResponse).toHaveBeenCalledWith('remote_desktop.grant_consent', {
    args: {
      intent: 'remote_desktop_session',
      input_control: true,
    },
  })
  expect(mocks.invokeMediaUnary).toHaveBeenCalledWith(
    'remote_desktop.create_session',
    expect.objectContaining({
      args: expect.objectContaining({
        mode: 'interactive',
        input_policy: {
          keyboard_enabled: true,
          pointer_enabled: true,
          clipboard_enabled: false,
          file_drop_enabled: false,
        },
      }),
    }),
  )
  expect(mocks.invokeMediaStream).toHaveBeenCalledWith(
    'remote_desktop.watch_events',
    expect.objectContaining({
      subjectURA: screenResource.resource_ura,
      args: { session_id: 'rd-1', session_token: 'session-token' },
      timeoutMs: 0,
    }),
    expect.anything(),
  )
})

it('keeps remote desktop consent and session input policy view-only when interactive is disabled', async () => {
  expect(mocks.invokeMediaUnaryResponse).toHaveBeenCalledWith('remote_desktop.grant_consent', {
    args: {
      intent: 'remote_desktop_session',
      input_control: false,
    },
  })
  expect(mocks.invokeMediaUnary).toHaveBeenCalledWith(
    'remote_desktop.create_session',
    expect.objectContaining({
      args: expect.objectContaining({
        mode: 'view_only',
        input_policy: {
          keyboard_enabled: false,
          pointer_enabled: false,
          clipboard_enabled: false,
          file_drop_enabled: false,
        },
      }),
    }),
  )
})

it('surfaces remote desktop recovery events from the session watcher', async () => {
  expect(useMediaChannelStore.getState().entries[key].webrtcStatus).toContain('input blocked')
  expect(useMediaChannelStore.getState().entries[key].webrtcStatus).toContain('input_injection_unavailable')
  expect(useMediaChannelStore.getState().entries[key].webrtcStatus).toContain('input rejected')
  expect(useMediaChannelStore.getState().entries[key].webrtcStatus).toContain('stale_pointer_target_geometry')
  expect(useMediaChannelStore.getState().entries[key].webrtcStatus).toContain('session needs retry')
  expect(useMediaChannelStore.getState().entries[key].webrtcStatus).toContain('permission was revoked')
  expect(useMediaChannelStore.getState().entries[key].session.terminalReceipt.reasonCode).toBe('target_permission_revoked')
  expect(useMediaChannelStore.getState().entries[key].session?.sessionToken).toBeUndefined()
})

it('surfaces RemoteApp input permission results from request_permission', async () => {
  expect(entry.webrtcStatus).toContain('Accessibility input permission requested but still unavailable')
})

it('checks RemoteApp host permissions without target-scoped subject', async () => {
  expect(mocks.invokeMediaUnary).toHaveBeenCalledWith('remote_desktop.permission_status', {
    args: {},
  })
  expect(entry.webrtcStatus).toContain('Accessibility input permission is not granted')
  expect(entry.error).toBeUndefined()
})

it('fails closed instead of queueing RemoteApp input behind RTC data-channel backpressure', async () => {
  expect(store.rdSendInput(key, { type: 'pointer', action: 'move', target_geometry_revision: 7 })).toBe(false)
  expect(entry.webrtcStatus).toContain('input backpressure')
})

it('includes RemoteApp input client sequence telemetry', async () => {
  expect(JSON.parse(inputChannel.sent[0])).toMatchObject({ client_sequence: 1 })
})

it('preserves and rebinds remote desktop sessions across device offline resume', async () => {
  expect(useMediaChannelStore.getState().entries[key].session.sessionId).toBe('rd-1')
  expect(useMediaChannelStore.getState().entries[key].session.sessionToken).toBe('session-token')
  expect(mocks.invokeMediaUnary).toHaveBeenCalledWith('remote_desktop.show_session', expect.anything())
  expect(mocks.invokeMediaUnary).toHaveBeenCalledWith('remote_desktop.set_description', expect.anything())
  expect(mocks.invokeMediaStream).toHaveBeenCalledWith('remote_desktop.watch_events', expect.anything(), expect.anything())
})
TS

cat >"$FRONTEND_SRC/components/easynet/DeviceMediaAccess.test.tsx" <<'TSX'
it('keeps base media controls available when remote desktop target refresh fails', async () => {
  expect(screen.getByRole('button', { name: /Camera/i })).not.toBeDisabled()
  expect(screen.getByRole('button', { name: /Remote desktop/i })).toBeDisabled()
})

it('runs the remote desktop UI flow from target picker through session end', async () => {
  expect(mocks.invokeAbility).toHaveBeenCalledWith(expect.objectContaining({
    ability: 'remote_desktop.grant_consent',
    subject_ura: screenResource.resource_ura,
    arguments: expect.objectContaining({
      intent: 'remote_desktop_session',
      input_control: true,
    }),
  }))
  expect(mocks.invokeAbility).toHaveBeenCalledWith(expect.objectContaining({
    ability: 'remote_desktop.create_session',
    subject_ura: screenResource.resource_ura,
    arguments: expect.objectContaining({
      mode: 'interactive',
      input_policy: {
        keyboard_enabled: true,
        pointer_enabled: true,
        clipboard_enabled: false,
        file_drop_enabled: false,
      },
    }),
  }))
  expect(mocks.invokeAbilityStream).toHaveBeenCalledWith(expect.objectContaining({
    ability: 'remote_desktop.watch_events',
    subject_ura: screenResource.resource_ura,
  }), expect.anything())
  expect(mocks.invokeAbility).toHaveBeenCalledWith(expect.objectContaining({
    ability: 'remote_desktop.end_session',
    subject_ura: screenResource.resource_ura,
  }))
})

it('surfaces daemon remote desktop input readiness in session details', async () => {
  expect(screen.getByText('input interactive->view_only · input_injection_unavailable')).toBeInTheDocument()
})

it('surfaces daemon remote desktop terminal receipts in session details', async () => {
  expect(screen.getByText('terminal caller_ended #9')).toBeInTheDocument()
})

it('does not end a remote desktop session when device presence drops offline', async () => {
  expect(rdEnd).not.toHaveBeenCalled()
})
TSX

CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

perl -0pi -e "s/      input_control: sessionInputIntent\\.inputControlRequested,\\n//" \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted grant_consent without input_control intent binding" >&2
  exit 1
fi
perl -0pi -e "s/      intent: 'remote_desktop_session',\\n/      intent: 'remote_desktop_session',\\n      input_control: sessionInputIntent.inputControlRequested,\\n/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e "s/mode: sessionInputIntent\\.mode,/mode: entry.interactive ? 'interactive' : 'view_only',/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted create_session mode outside the shared input intent" >&2
  exit 1
fi
perl -0pi -e "s/mode: entry\\.interactive \\? 'interactive' : 'view_only',/mode: sessionInputIntent.mode,/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e "s/input_policy: sessionInputIntent\\.inputPolicy,/input_policy: { keyboard_enabled: entry.interactive, pointer_enabled: entry.interactive, clipboard_enabled: false, file_drop_enabled: false },/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted create_session input_policy outside the shared input intent" >&2
  exit 1
fi
perl -0pi -e "s/input_policy: \\{ keyboard_enabled: entry\\.interactive, pointer_enabled: entry\\.interactive, clipboard_enabled: false, file_drop_enabled: false \\},/input_policy: sessionInputIntent.inputPolicy,/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

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

perl -0pi -e "s/mode: sessionInputIntent\\.mode,/mode: sessionInputIntent.mode,\\n      subject_ura: resource.resource_ura,/" \
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

cp "$FRONTEND_SRC/store/media-channel-store.ts" \
  "$FRONTEND_SRC/store/media-channel-store.ts.good"
perl -0pi -e 's/input_permission/permission/g' \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted request_permission without input_permission parsing" >&2
  exit 1
fi
mv "$FRONTEND_SRC/store/media-channel-store.ts.good" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

cp "$FRONTEND_SRC/store/media-channel-store.ts" \
  "$FRONTEND_SRC/store/media-channel-store.ts.good"
perl -0pi -e 's/Accessibility input permission/Input permission/g' \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted request_permission status without Accessibility input wording" >&2
  exit 1
fi
mv "$FRONTEND_SRC/store/media-channel-store.ts.good" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e 's/surfaces RemoteApp input permission results from request_permission/surfaces RemoteApp screen permission results from request_permission/' \
  "$FRONTEND_SRC/store/media-channel-store.test.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted tests without input permission request coverage" >&2
  exit 1
fi
perl -0pi -e 's/surfaces RemoteApp screen permission results from request_permission/surfaces RemoteApp input permission results from request_permission/' \
  "$FRONTEND_SRC/store/media-channel-store.test.ts"

perl -0pi -e 's/(remote_desktop\.permission_status'\''[\s\S]*?deviceUra: entry\.deviceUra,\n)/$1      subjectURA: selectedTarget.resource_ura,\n/s' \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted target-scoped permission_status" >&2
  exit 1
fi
perl -0pi -e "s/\\n      subjectURA: selectedTarget\\.resource_ura,//" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e 's/remoteDesktopPermissionStatusResult\(result\)/result/' \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted permission_status without structured preflight formatting" >&2
  exit 1
fi
perl -0pi -e 's/return result/return remoteDesktopPermissionStatusResult(result)/' \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e 's/checks RemoteApp host permissions without target-scoped subject/checks generic RemoteApp permissions/' \
  "$FRONTEND_SRC/store/media-channel-store.test.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted tests without permission_status host-local coverage" >&2
  exit 1
fi
perl -0pi -e 's/checks generic RemoteApp permissions/checks RemoteApp host permissions without target-scoped subject/' \
  "$FRONTEND_SRC/store/media-channel-store.test.ts"

perl -0pi -e 's/expect\(entry\.error\)\.toBeUndefined\(\)/expect(entry.error).toContain('\''permission'\'')/' \
  "$FRONTEND_SRC/store/media-channel-store.test.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted tests that treat permission_status as session error" >&2
  exit 1
fi
perl -0pi -e 's/expect\(entry\.error\)\.toContain\('\''permission'\''\)/expect(entry.error).toBeUndefined()/' \
  "$FRONTEND_SRC/store/media-channel-store.test.ts"

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

perl -0pi -e 's/  const targetGeometryRevision = entry\.session\?\.targetTracking\?\.targetGeometryRevision\n//' \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted pointer frames without session target geometry revision binding" >&2
  exit 1
fi
perl -0pi -e "s/  const pointerFrame = \\{/  const targetGeometryRevision = entry.session?.targetTracking?.targetGeometryRevision\\n  const pointerFrame = {/" \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx"

perl -0pi -e 's/    target_geometry_revision: targetGeometryRevision,\n//' \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted pointer frames without target_geometry_revision payload" >&2
  exit 1
fi
perl -0pi -e "s/    type: 'pointer',/    type: 'pointer',\\n    target_geometry_revision: targetGeometryRevision,/" \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx"

perl -0pi -e "s/if \\(pc\\.connectionState === 'connected'\\) updateWebRtcStatus\\(\\)/if (pc.connectionState === 'connected') reportClientMediaState(key, 'presenting')/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted peer-connected client presentation report" >&2
  exit 1
fi
perl -0pi -e "s/if \\(pc\\.connectionState === 'connected'\\) reportClientMediaState\\(key, 'presenting'\\)/if (pc.connectionState === 'connected') updateWebRtcStatus()/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e "s/'remote_desktop\\.watch_events'/'remote_desktop.show_session'/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted missing watch_events session stream" >&2
  exit 1
fi
perl -0pi -e "s/'remote_desktop\\.show_session'/'remote_desktop.watch_events'/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e "s/event\\.eventType === 'TARGET_PERMISSION_REVOKED'/event.eventType === 'TARGET_PERMISSION_IGNORED'/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted missing permission-revoked recovery handling" >&2
  exit 1
fi
perl -0pi -e "s/event\\.eventType === 'TARGET_PERMISSION_IGNORED'/event.eventType === 'TARGET_PERMISSION_REVOKED'/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e "s/event\\.eventType === 'INPUT_FRAME_REJECTED'/event.eventType === 'INPUT_FRAME_IGNORED'/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted missing input rejection event handling" >&2
  exit 1
fi
perl -0pi -e "s/event\\.eventType === 'INPUT_FRAME_IGNORED'/event.eventType === 'INPUT_FRAME_REJECTED'/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e "s/event\\.eventType === 'INPUT_CHANNEL_OPENED'/event.eventType === 'INPUT_CHANNEL_IGNORED'/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted missing input activation block event handling" >&2
  exit 1
fi
perl -0pi -e "s/event\\.eventType === 'INPUT_CHANNEL_IGNORED'/event.eventType === 'INPUT_CHANNEL_OPENED'/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e "s/status: 'remote desktop input rejected \\(stale_pointer_target_geometry\\)',\\n      closeLocalTransport: false/status: 'remote desktop input rejected (stale_pointer_target_geometry)',\\n      closeLocalTransport: true/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted input rejection closing media transport" >&2
  exit 1
fi
perl -0pi -e "s/status: 'remote desktop input rejected \\(stale_pointer_target_geometry\\)',\\n      closeLocalTransport: true/status: 'remote desktop input rejected (stale_pointer_target_geometry)',\\n      closeLocalTransport: false/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e "s/webrtcStatus: 'device offline; remote desktop session preserved for reconnect',/session: null,\\n      webrtcStatus: 'device offline; remote desktop session preserved for reconnect',/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted device-offline RemoteApp session clearing" >&2
  exit 1
fi
perl -0pi -e "s/\\n      session: null,//" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e "s/subjectURA: session\\.subjectUra,/subjectURA: entry.subjectUra,/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted RemoteApp offline resume without show_session validation" >&2
  exit 1
fi
perl -0pi -e "s/subjectURA: entry\\.subjectUra,/subjectURA: session.subjectUra,/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e "s/startWebRtc\\(key, view, \\{ endSessionOnTransportFailure: false \\}\\)/startWebRtc(key, view, { endSessionOnTransportFailure: true })/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted offline resume transport failure ending the daemon session" >&2
  exit 1
fi
perl -0pi -e "s/startWebRtc\\(key, view, \\{ endSessionOnTransportFailure: true \\}\\)/startWebRtc(key, view, { endSessionOnTransportFailure: false })/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e "s/  return remoteTargetReady/  if (online === false) rdEnd(channelKey)\\n  return remoteTargetReady/" \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted UI presence-offline RemoteApp session end" >&2
  exit 1
fi
perl -0pi -e "s/  if \\(online === false\\) rdEnd\\(channelKey\\)\\n//" \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx"

perl -0pi -e "s/runs the remote desktop UI flow from target picker through session end/runs an incomplete remote desktop UI flow/" \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.test.tsx"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted missing full remote desktop UI flow test" >&2
  exit 1
fi
perl -0pi -e "s/runs an incomplete remote desktop UI flow/runs the remote desktop UI flow from target picker through session end/" \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.test.tsx"

perl -0pi -e 's/remoteDesktopInputReadinessLabel\(session\)/`input ${session.inputPolicy}`/' \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted session details without daemon input_readiness rendering" >&2
  exit 1
fi
perl -0pi -e 's/`input \$\{session\.inputPolicy\}`/remoteDesktopInputReadinessLabel(session)/' \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx"

perl -0pi -e 's/readiness\.blockedReason/readiness.reasonBlocked/g' \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted input readiness details without blocked_reason" >&2
  exit 1
fi
perl -0pi -e 's/readiness\.reasonBlocked/readiness.blockedReason/g' \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.tsx"

perl -0pi -e "s/surfaces daemon remote desktop input readiness in session details/surfaces only requested input policy in session details/" \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.test.tsx"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted missing input readiness details test" >&2
  exit 1
fi
perl -0pi -e "s/surfaces only requested input policy in session details/surfaces daemon remote desktop input readiness in session details/" \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.test.tsx"

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

perl -0pi -e 's/JSON\.stringify\(\{ \.\.\.frame, sent_at_ms: Date\.now\(\) \}\)/JSON.stringify(frame)/' \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted input send without client timestamp metadata" >&2
  exit 1
fi
perl -0pi -e 's/JSON\.stringify\(frame\)/JSON.stringify({ ...frame, sent_at_ms: Date.now() })/' \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e "s/    inputReadiness,\\n//" \
  "$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted missing daemon input_readiness session projection" >&2
  exit 1
fi
perl -0pi -e "s/    inputPolicy: 'view-only',/    inputPolicy: 'view-only',\\n    inputReadiness,/" \
  "$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"

perl -0pi -e "s/  const value = objectField\\(result, 'input_readiness'\\)/  const value = objectField(result, 'input_policy')/" \
  "$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted input readiness parser that does not read daemon input_readiness" >&2
  exit 1
fi
perl -0pi -e "s/  const value = objectField\\(result, 'input_policy'\\)/  const value = objectField(result, 'input_readiness')/" \
  "$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"

perl -0pi -e "s/    terminalReceipt: remoteDesktopTerminalReceiptFromResult\\(result\\),\\n//" \
  "$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted missing daemon terminal_receipt session projection" >&2
  exit 1
fi
perl -0pi -e "s/    inputReadiness,\\n/    inputReadiness,\\n    terminalReceipt: remoteDesktopTerminalReceiptFromResult(result),\\n/" \
  "$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"

perl -0pi -e "s/session: view \\? \\{ \\.\\.\\.view, sessionToken: undefined \\} : null/session: null/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted end_session clearing terminal session view" >&2
  exit 1
fi
perl -0pi -e "s/session: null/session: view ? { ...view, sessionToken: undefined } : null/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e "s/\\nfunction guardTerminalRemoteDesktopSessionPatch[\\s\\S]*?\\n}\\n\\ntype RemoteDesktopSessionInputIntent/\\ntype RemoteDesktopSessionInputIntent/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted missing terminal session stale async guard" >&2
  exit 1
fi
perl -0pi -e "s/import \\{ remoteDesktopInputFrameAllowed, remoteDesktopSessionTerminal \\} from '@\\/lib\\/api\\/remote-desktop-protocol'\\n\\n/import { remoteDesktopInputFrameAllowed, remoteDesktopSessionTerminal } from '@\\/lib\\/api\\/remote-desktop-protocol'\\n\\nfunction guardTerminalRemoteDesktopSessionPatch(prev: Entry, patch: Partial<Entry>): Partial<Entry> {\\n  const current = prev.session\\n  const next = patch.session\\n  if (current \\&\\& next \\&\\& remoteDesktopSessionTerminal(current) \\&\\& !remoteDesktopSessionTerminal(next)) {\\n    return { ...patch, session: current, attached: false }\\n  }\\n  return patch\\n}\\n\\n/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"

perl -0pi -e "s/surfaces daemon remote desktop terminal receipts in session details/surfaces only closed remote desktop state/" \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.test.tsx"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted missing terminal receipt details test" >&2
  exit 1
fi
perl -0pi -e "s/surfaces only closed remote desktop state/surfaces daemon remote desktop terminal receipts in session details/" \
  "$FRONTEND_SRC/components/easynet/DeviceMediaAccess.test.tsx"

perl -0pi -e 's/    if \(view\.inputReadiness\.interactiveReady !== true\) return false/    if (false) return false/' \
  "$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted input readiness gating that does not fail closed" >&2
  exit 1
fi
perl -0pi -e 's/    if \(false\) return false/    if (view.inputReadiness.interactiveReady !== true) return false/' \
  "$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"

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

perl -0pi -e 's/productionReady: productionReadiness\?\.ready === true/productionReady: productionGate?.ready === true || mediaBackends.some(isRemoteDesktopProductionBackend)/' \
  "$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted production readiness derived from capability gates" >&2
  exit 1
fi
perl -0pi -e 's/productionReady: productionGate\?\.ready === true \|\| mediaBackends\.some\(isRemoteDesktopProductionBackend\)/productionReady: productionReadiness?.ready === true/' \
  "$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"

perl -0pi -e 's/productionReady: productionReadiness\?\.ready === true/productionReady: result?.production_media_ready === true || productionReadiness?.ready === true/' \
  "$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted legacy production_media_ready OR predicate" >&2
  exit 1
fi
perl -0pi -e 's/productionReady: result\?\.production_media_ready === true \|\| productionReadiness\?\.ready === true/productionReady: productionReadiness?.ready === true/' \
  "$FRONTEND_SRC/lib/api/remote-desktop-protocol.ts"

echo "test_check_remoteapp_frontend_invocation_boundary.sh: all cases passed"
