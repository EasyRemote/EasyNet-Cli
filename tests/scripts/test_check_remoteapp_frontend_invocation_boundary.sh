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
  "$FRONTEND_SRC/pages/easynet"

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
export function DeviceMediaAccess() {
  const baseRuntimeReady = online === true && !resourceRuntimeOffline
  const remoteTargetReady = baseRuntimeReady && !remoteTargetError
  const remoteTargetData = listRemoteDesktopTargets()
  const result = invokeMediaUnary('resource.refresh_remote_targets', {})
  const screenResources = remoteTargetData.resources
  const screenResource = screenResources.find((resource) => resource.resource_ura === selectedScreenURA)
  if (selectedScreenURA && !screenResources.some((resource) => resource.resource_ura === selectedScreenURA)) {
    setSelectedScreenURA(undefined)
  }
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

perl -0pi -e "s/mode: 'view_only',/mode: 'view_only',\\n      subject_ura: resource.resource_ura,/" \
  "$FRONTEND_SRC/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp frontend checker accepted create_session args.subject_ura" >&2
  exit 1
fi
perl -0pi -e "s/\\n      subject_ura: resource\\.resource_ura,//" \
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

echo "test_check_remoteapp_frontend_invocation_boundary.sh: all cases passed"
