# RemoteApp Product Readiness Audit — 2026-08-22

Status: product closure incomplete.

This audit separates verified targeted-session architecture from full
interactive RemoteApp product readiness. Passing the current boundary gates
does not mean RemoteApp is product-complete.

The machine-readable gate source for this audit is
`docs/design/remoteapp-product-readiness-matrix.json`. The Markdown table below
is explanatory; the JSON matrix is the product-closure status contract consumed
by `tools/scripts/check-remoteapp-product-closure-audit.sh`.

## Architecture boundary

RemoteApp remains an EasyNet-Cli device plugin capability:

- User Principal is the caller/accountability root.
- Device is host/sponsor/key custodian, not the public callee.
- SystemAgent owns device-native `remote_desktop.*` AbilityDescriptors.
- RemoteDesktopPlugin is the AbilityImpl.
- Display/window/application Resource URA is the Invocation subject.
- WebRTC/native media is a session transport, not a second Invocation model.

## Current evidence baseline

The following gates prove useful boundaries and regression constraints:

- `tools/scripts/check-remoteapp-target-binding-boundary.sh`
- `tools/scripts/check-remoteapp-lifecycle-input-boundary.sh`
- `tools/scripts/check-remoteapp-e2e-acceptance-boundary.sh`
- `tools/scripts/check-remoteapp-frontend-invocation-boundary.sh`
- `tools/scripts/check-remoteapp-input-consent-boundary.sh`
- `tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh`
- `tools/scripts/check-remoteapp-performance-boundary.sh`
- `tools/scripts/check-remoteapp-picker-subject-boundary.sh`
- `tools/scripts/check-remoteapp-session-subject-boundary.sh`
- `tools/scripts/frontend-remoteapp-product-flow-e2e.sh`
- `tools/scripts/remoteapp-cross-device-product-smoke.sh`

They prove that current source contracts preserve target binding, subject
placement, view-only safety, source-level E2E harnesses, and performance
boundaries. The frontend invocation boundary also proves that the browser
surface drives the picker-to-session-to-end UI path, starts
`remote_desktop.watch_events` after negotiated WebRTC setup, and maps
degraded/permission-revoked session events into recovery UI/transport state.
It also proves the frontend session projection consumes daemon
`input_readiness` and makes input sending fail closed from that runtime
readiness when present, rather than relying only on legacy `input_policy`.
It also proves the frontend session-details surface renders daemon-projected
target recovery state: target status, target failure reason, and
`frontend_action` guidance such as `refresh_targets` are visible when a selected
window/application target is lost. The action row also exposes a `Refresh
targets` CTA that refetches the daemon target inventory when that action is
requested, rather than leaving the recovery as a non-executable message.
The input-consent boundary proves that media/session consent no longer
implicitly authorizes pointer/keyboard control: `grant_consent` must explicitly
mint an input-control scoped ticket, `create_session` must consume that scope,
display targets may then project `display_global` input scope, and
window/application targets remain `view_only` until target-scoped dispatch is
safe.
The frontend product-flow script is a runnable product-flow harness entrypoint:
with an explicit `--run`, it first verifies Hub API reachability, then product
runtime readiness for daemon control/invocation, then composes frontend
typecheck/UI flow coverage with host permission-subject, target-freshness,
decoded-frame, and view-only input E2E harnesses. A skipped/self-test report
from that entrypoint is only harness evidence, not product completion.
The frontend session-details surface now also renders daemon route state; a
host-only WebRTC route is visible as `route host_only · no NAT/relay`, so
transport presence is not confused with production NAT/relay readiness.
The same surface now renders a compact media quality summary from daemon/browser
stats, including bitrate, outbound FPS, total drops, and RTP backpressure.
The latest local run,
`target/e2e/frontend-remoteapp-product-flow/20260822-044248-69775/report.md`,
passed the bounded single-machine product-flow bundle after the local Hub was
restarted with the paired `localhost` realm and the device connection-state
projector preserved `hub_api_endpoint` across the
`FRONTEND_CONNECTED` projection.
The cross-device smoke entrypoint composes the existing two-node EasyRemote CLI
E2E and synthetic media/bidi Docker E2E. Its evidence scope is intentionally
narrow: governed Hub routing, cross-device ability visibility/invocation, and
synthetic stream/bidi carrier receipt chains. It explicitly does not prove real
OS window/application capture, input injection, host audio, NAT/TURN deployment,
or frontend browser rendering.
The latest local cross-device run failed at the two-node routing step: the
provider became visible as an online federated device, but the caller's
user-scoped `service/alice.pages` owner projection was rejected by the Hub with
`accepted_count=0, expected_count=5`. That failure is upstream of RemoteApp
target inventory/media and keeps cross-device product readiness partial.
They do not prove every operating system, network topology, input mode, codec
path, and frontend lifecycle is product-ready.

## Product closure matrix

| Requirement | Current status | Evidence that exists | Evidence still required before product-complete |
|---|---|---|---|
| Application/window selection and stable capture across macOS/Windows/Linux | Partial | macOS ScreenCaptureKit target model and host decoded-frame harnesses; macOS application capture passes uncommitted same-app same-display windows as `exceptingWindows` so committed window-set sessions do not widen to every same-app window; non-macOS app/window target observation fails closed; frontend session details surface daemon target loss reason/recovery action and the action row can execute `refresh_targets` by refetching target inventory | Real macOS, Windows, and Linux host E2E reports for display/window/application; Windows/Linux native capture plugins or explicit product unsupported state |
| Mouse/keyboard input injection is controllable, low-latency, and permission-correct | Incomplete | App/window sessions downgrade to `view_only`; pointer/key frames are policy-gated; clipboard/file-drop are unsupported; session views now expose `input_readiness` with requested/effective mode, interactive readiness, and blocked reason; frontend session details separately surface input scope plus pointer/keyboard enablement, such as `input scope display_global · pointer+keyboard` or `input scope display_global · no controls`; frontend input sending consumes daemon `input_readiness` and fails closed before sending pointer/key frames; frontend rejects missing or stale pointer `target_geometry_revision` before WebRTC data-channel send; frontend now refuses to enqueue RemoteApp input when the RTC data-channel backlog exceeds the explicit input bound and attaches monotonic `client_sequence` plus `sent_at_ms` telemetry to accepted frames; daemon pointer/key input schema accepts frontend `sent_at_ms` and `client_sequence` metadata and preserves `client_sent_at_ms`/`client_sequence` in input applied/rejected events; diagnostic InvokeBidi input responses preserve the same telemetry for probe correlation, including `target_input_not_ready`; frontend `watch_events` surfaces daemon input activation blocks and `INPUT_FRAME_REJECTED` reasons without closing media transport; display interactive sessions require an explicit input-control consent ticket before resolving `display_global` input scope; target tracker input loss projects `target_input_not_ready`; OS accessibility absence still reports `input_injection_unavailable`; `remote_desktop.request_permission` contract and frontend status expose Accessibility/input-injection permission alongside Screen Recording, and the UI offers `Request permission` from daemon input-injection blockers | Focus validation, coordinate mapping, target epoch checks on execution path, OS permission checks, latency measurements, and successful input injection E2E |
| Audio/video codec, frame rate, bitrate adaptation, and drop policy are product-ready | Partial | macOS H.264/WebRTC path, VideoToolbox descriptor, adaptive bitrate helper, queue/drop boundary tests, session/device capability view explicitly reports `host_audio_not_implemented`, frontend session details surface `host_audio_not_implemented` from daemon audio readiness projection, frontend session details surface media bitrate/FPS/drop/backpressure stats, ABI v8 raw stream metadata contract preserves canonical lifecycle fields without base64 payload projection, and release packaging/install gates ship the v8 raw-stream export allowlist beside the base v7 ABI contract | End-to-end codec negotiation reports, audio path, frame-rate/bitrate soak under load, degraded network/drop policy E2E |
| Multi-window/multi-application independent tracking works as an execution effect | Partial | Target tracker state machine, move/resize/loss/rebind events, same-display application window-set rebind, ScreenCaptureKit `exceptingWindows` for uncommitted same-app windows; frontend session details surface target tracking recovery state and a Refresh targets CTA instead of opaque failure | Multi-display `MultiAppSurface` or explicit product unsupported report; real app/window churn E2E with independent tracked streams |
| Disconnect/reconnect, session resume, consent revoke, cancel, timeout are complete | Partial | Lease monitor, refresh/end session, explicit `end_session` and lease-timeout `terminal_receipt` projection, frontend retention of closed `end_session` views with terminal receipt and cleared session token, permission revocation terminates the daemon session with a `terminal_receipt` and frontend terminal sync, target loss and transport failure taxonomy, frontend watch_events recovery handling for degraded and permission-revoked sessions, frontend exposes `Retry session` for daemon retry-session guidance, terminal receipt retention no longer blocks new `create_session`, frontend preserves non-terminal RemoteApp sessions across device-offline presence drops and rebinds through `show_session` + WebRTC restart, canonical SDK cancel/timeout semantics | Long-outage and network transport reconnect handoff E2E, consent revoke termination E2E, canonical Axon cancel/timeout receipt-chain evidence for RemoteApp abilities, crash/restart recovery E2E |
| NAT/relay/WebRTC/direct fallback network paths are verified | Partial | Typed host/STUN/TURN/EasyNet relay route evidence, source-level provider gates, daemon `client_ice_servers` projection, frontend `RTCPeerConnection` consumption, and frontend route-state visibility such as `route host_only · no NAT/relay` | Real direct, STUN srflx, TURN relay, EasyNet relay deployment reports with credentials redacted and reachability verified |
| Frontend UI can discover, authorize, start, display, control, and end session | Partial | Frontend subject boundary, dedicated surface gates, component coverage for picker → permission_status preflight → consent → create → WebRTC attach → watch_events → end, denied permission_status remains in the picker with Request permission recovery instead of becoming a session error, target-scoped WebRTC lifecycle unit coverage, watch_events recovery-state coverage, daemon `input_readiness`, executable target refresh recovery, executable retry-session recovery, and `terminal_receipt` session detail projection coverage, permission-revoked terminal sync coverage, and product-flow harness entrypoint for combined frontend/host evidence | Browser/Tauri E2E for full user flow with real backend/runtime: picker → permission → consent → create_session → WebRTC attach → watch_events recovery → input/control → end_session |
| Cross-device E2E smoke/regression exists beyond local provider boundary | Partial | `remoteapp-cross-device-product-smoke.sh` composes Docker two-node routing and synthetic media/bidi carrier gates; host-local decoded-frame scripts cover local capture/render decode | RemoteApp-specific two-device or equivalent network namespace E2E with remote target inventory, remote WebRTC/media from actual display/window/application capture, input policy, and teardown evidence |

## Product-complete definition

RemoteApp may be called product-complete only when every row above has current
authoritative evidence. A source-contract checker, unit test, local provider
benchmark, or SPEC statement is insufficient for a row whose scope is real OS,
real frontend, real network, or cross-device behavior.

Until then, the correct status is:

```text
RemoteApp targeted-session architecture: implemented enough for guarded
macOS-focused app/window/display media work.

RemoteApp interactive desktop product: incomplete.
```

## Next implementation batches

1. Frontend full-flow E2E: browser/Tauri picker → permission → consent →
   create_session → WebRTC attach → watch_events → end_session.
2. Product input batch: focus-safe pointer/keyboard injection with target epoch
   validation and permission proof.
3. Network batch: direct/STUN/TURN/EasyNet relay deployment matrix and degraded
   route UI evidence.
4. Cross-platform capture batch: Windows/Linux explicit implementation or
   product-level unsupported state with UI affordance.
5. Recovery batch: reconnect/session resume, revoke, cancel, timeout,
   crash/restart recovery.
