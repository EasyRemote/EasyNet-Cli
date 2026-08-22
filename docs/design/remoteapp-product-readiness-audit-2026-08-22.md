# RemoteApp Product Readiness Audit — 2026-08-22

Status: product closure incomplete.

This audit separates verified targeted-session architecture from full
interactive RemoteApp product readiness. Passing the current boundary gates
does not mean RemoteApp is product-complete.

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
- `tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh`
- `tools/scripts/check-remoteapp-performance-boundary.sh`
- `tools/scripts/check-remoteapp-picker-subject-boundary.sh`
- `tools/scripts/check-remoteapp-session-subject-boundary.sh`
- `tools/scripts/frontend-remoteapp-product-flow-e2e.sh`

They prove that current source contracts preserve target binding, subject
placement, view-only safety, source-level E2E harnesses, and performance
boundaries. The frontend invocation boundary also proves that the browser
surface drives the picker-to-session-to-end UI path, starts
`remote_desktop.watch_events` after negotiated WebRTC setup, and maps
degraded/permission-revoked session events into recovery UI/transport state.
The frontend product-flow script is a runnable product-flow harness entrypoint:
with an explicit `--run`, it composes frontend typecheck/UI flow coverage with
host permission-subject, target-freshness, decoded-frame, and view-only input
E2E harnesses. A skipped/self-test report from that entrypoint is only harness
evidence, not product completion.
They do not prove every operating system, network topology, input mode, codec
path, and frontend lifecycle is product-ready.

## Product closure matrix

| Requirement | Current status | Evidence that exists | Evidence still required before product-complete |
|---|---|---|---|
| Application/window selection and stable capture across macOS/Windows/Linux | Partial | macOS ScreenCaptureKit target model and host decoded-frame harnesses; non-macOS app/window target observation fails closed | Real macOS, Windows, and Linux host E2E reports for display/window/application; Windows/Linux native capture plugins or explicit product unsupported state |
| Mouse/keyboard input injection is controllable, low-latency, and permission-correct | Incomplete by design | App/window sessions downgrade to `view_only`; pointer/key frames are policy-gated; clipboard/file-drop are unsupported | Focus validation, coordinate mapping, target epoch checks, OS permission checks, latency measurements, and successful input injection E2E |
| Audio/video codec, frame rate, bitrate adaptation, and drop policy are product-ready | Partial | macOS H.264/WebRTC path, VideoToolbox descriptor, adaptive bitrate helper, queue/drop boundary tests | End-to-end codec negotiation reports, audio path, frame-rate/bitrate soak under load, degraded network/drop policy E2E |
| Multi-window/multi-application independent tracking works as an execution effect | Partial | Target tracker state machine, move/resize/loss/rebind events, same-display application window-set rebind | Multi-display `MultiAppSurface` or explicit product unsupported report; real app/window churn E2E with independent tracked streams |
| Disconnect/reconnect, session resume, consent revoke, cancel, timeout are complete | Partial | Lease monitor, refresh/end session, target loss and transport failure taxonomy, frontend watch_events recovery handling for degraded and permission-revoked sessions, canonical SDK cancel/timeout semantics | Session resume after transport loss, reconnect handoff, consent revoke termination E2E, cancel/timeout receipts, crash/restart recovery E2E |
| NAT/relay/WebRTC/direct fallback network paths are verified | Partial | Typed host/STUN/TURN/EasyNet relay route evidence and source-level provider gates | Real direct, STUN srflx, TURN relay, EasyNet relay deployment reports with credentials redacted and reachability verified |
| Frontend UI can discover, authorize, start, display, control, and end session | Partial | Frontend subject boundary, dedicated surface gates, component coverage for picker → consent → create → WebRTC attach → watch_events → end, target-scoped WebRTC lifecycle unit coverage, watch_events recovery-state coverage, and product-flow harness entrypoint for combined frontend/host evidence | Browser/Tauri E2E for full user flow with real backend/runtime: picker → permission → consent → create → WebRTC attach → watch_events recovery → input/control → end |
| Cross-device E2E smoke/regression exists beyond local provider boundary | Missing as product proof | Docker media/bidi source contract; host-local decoded-frame scripts | Two real devices or equivalent network namespace E2E with Hub routing, remote target inventory, remote WebRTC/media, and teardown evidence |

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
