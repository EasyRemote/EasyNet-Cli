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
- `tools/scripts/check-remoteapp-main-crate-implementation-tests.sh`

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
The main-crate implementation test gate,
`tools/scripts/check-remoteapp-main-crate-implementation-tests.sh`, proves that
RemoteApp app/window target observation, non-macOS app/window fail-closed
behavior, direct WebRTC app/window display-fallback rejection, native plugin
platform catalogue state, and current-session input policy tests are executed
through the daemon-embedded main EasyNet crate. It also records that the
standalone `easynet-plugin-remote-desktop` crate is a provider/export shim, so
a zero-test standalone package result is not implementation evidence.
The device capability projection also separates
`metadata.production_target_subjects` from
`metadata.diagnostic_target_subjects`: unavailable or permission-blocked
production backends do not advertise app/window/application as current
production subjects, while the xcap diagnostic fallback remains display-only.
That projection uses the same runtime native backend descriptor as
`production_gate_view`, so Screen Recording permission denial and non-macOS
`not_installed` state are not hidden by the compile-time macOS descriptor.
The lifecycle/input boundary gate and mutation tests now pin this projection
shape, including the runtime-native descriptor source, production-ready gate,
display-only diagnostic subjects, blocked reason, and production subject source.
The same device capability metadata now exposes `metadata.platform_support` for
macOS, Linux, and Windows. macOS target rows follow the native production gate,
Linux display is diagnostic-only, Linux window/application are unsupported, and
Windows display/window/application are unsupported until native backends exist.
It also exposes `metadata.input_control_support`: macOS display input follows
the runtime Accessibility/input-injection permission, macOS window/application
input remains unsupported until target-scoped dispatch is safe, and
Linux/Windows input injection is unsupported until native input backends exist.
The frontend product-flow script is a runnable product-flow harness entrypoint:
with an explicit `--run`, it first verifies Hub API reachability, then product
runtime readiness for daemon control/invocation, then composes frontend
typecheck/UI flow coverage with host permission-subject, target-freshness,
decoded-frame, and view-only input E2E harnesses. A skipped/self-test report
from that entrypoint is only harness evidence, not product completion.
The Browser/Tauri lifecycle verifier,
`tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh`, defines the
required artifact contract for a real frontend runner: proof mode
`real_browser_tauri_lifecycle`, `component_mock=false`,
`real_backend_runtime=true`, ordered picker/permission/consent/create/attach/
watch/media/media-pipeline-support/input/end/terminal-receipt steps, public
RemoteApp ability names, host-local `permission_status`, selected Resource URA
binding for session abilities, visible `media_pipeline_support`, and no
product-complete claim. Its self-test only proves the contract validator; a
live Browser/Tauri artifact is still required.
The frontend session-details surface now also renders daemon route state; a
host-only WebRTC route is visible as `route host_only · no NAT/relay`, so
transport presence is not confused with production NAT/relay readiness.
The same surface now renders a compact media quality summary from daemon/browser
stats, including bitrate, outbound FPS, total drops, and RTP backpressure.
It also renders daemon-projected `media_pipeline_support`, including video-only
scope, H.264 pipeline identity, bounded stale-frame drop policy, and product
blockers such as `host_audio_not_implemented`; this is frontend transparency,
not host-audio or degraded-network E2E evidence.
The network fallback verifier,
`tools/scripts/remoteapp-network-fallback-e2e.sh`, defines the live artifact
contract for real direct, STUN srflx, TURN relay, and EasyNet relay paths. It
requires connected WebRTC selected candidate-pair evidence, rendered media,
public RemoteApp session abilities, selected Resource URA subject binding,
credential redaction, and visible terminal receipts for every route scenario.
Its self-test proves only the artifact validator; it is not direct/STUN/TURN/
relay reachability evidence.
The cross-platform capture verifier,
`tools/scripts/remoteapp-cross-platform-capture-e2e.sh`, defines the live
artifact contract for macOS, Windows, and Linux display/window/application
capture. macOS must pass all three target kinds with rendered frames, exact
target binding, public RemoteApp session abilities, selected Resource URA
subject binding, and visible terminal receipts. Windows/Linux must either pass
the same capture scenarios or expose explicit product unsupported state without
creating a capture session or starting display fallback.
The input injection verifier,
`tools/scripts/remoteapp-input-injection-e2e.sh`, defines the live artifact
contract for focus-safe pointer/keyboard control. It requires real OS input
permission, `input_control` consent, `display_global` input scope, target
geometry revision binding, focus validation, coordinate mapping validation,
`INPUT_FRAME_APPLIED` events with `client_sequence` and `client_sent_at_ms`,
bounded host-applied latency, observed OS input effect, and visible terminal
receipt. Its self-test proves only the contract validator; it is not successful
OS input-injection evidence.
The media adaptation verifier,
`tools/scripts/remoteapp-media-adaptation-e2e.sh`, defines the live artifact
contract for the RemoteApp audio/video data plane. It requires baseline,
degraded-network, and backpressure scenarios with negotiated video codec,
payload content type, transport, requested/effective/measured FPS, target and
observed bitrate, keyframe cadence, bounded frame latency, real host audio,
bounded queue depth, explicit stale-frame drop policy, bitrate/FPS adaptation
or frame-drop evidence under impairment, rendered media after adaptation,
public RemoteApp session abilities, selected Resource URA subject binding, and
visible terminal receipts. Its self-test proves only the contract validator;
it is not codec, host-audio, soak, or degraded-network product evidence.
The multi-window tracking verifier,
`tools/scripts/remoteapp-multi-window-tracking-e2e.sh`, defines the live
artifact contract for independent app/window tracking as an execution effect.
It requires independent concurrent window streams with distinct Resource URAs,
session ids, stream ids, media source epochs, and frame source ids;
non-interleaved frames; ordered `TARGET_MOVED`/`TARGET_RESIZED` geometry churn
with increasing geometry revisions; same-display application window-set churn
with pending media rebind and `TARGET_REBOUND`; target loss with bounded
rebind or explicit rebind failure; multi-display application pass through
`MultiAppSurface` or explicit product unsupported state without capture start;
public RemoteApp session abilities; selected Resource URA subject binding; and
visible terminal receipts. Its self-test proves only the contract validator;
it is not real multi-window tracking evidence.
The crash/restart recovery verifier,
`tools/scripts/remoteapp-crash-restart-recovery-e2e.sh`, defines the live
artifact contract for deterministic RemoteApp recovery after daemon, plugin
worker, terminal-receipt, and stale-socket interruptions. It requires public
RemoteApp abilities, same-session recovery after daemon restart, recovered WAL/
idempotency/replay-guard/lock state, `show_session` validation, watch-events
and media reattachment, plugin worker/target monitor restart without minting a
new public session, original terminal receipt replay after crash during close,
explicit stale control/invocation socket cleanup, endpoint readiness, selected
Resource URA subject binding, and visible terminal receipts. Its self-test
proves only the contract validator; it is not real crash/restart recovery
evidence.
The latest live crash/restart probe,
`target/e2e/remoteapp-crash-restart-probe/20260822-223509-45956`, killed the
daemon with active RemoteApp window session `rd-crash-probe-45956` and then
restarted it. Public `remote_desktop.show_session` returned
`session_not_found` for the original session id, proving the historical runtime
did not rehydrate active RemoteApp sessions after daemon crash.
`RemoteDesktopRecoveryStore` / `RemoteDesktopRecoverySnapshot` now provide
daemon-local snapshots, lifecycle write-boundary persistence, and Stage 1
plugin startup rehydration into `RemoteDesktopSessionStore`. Rehydrated
non-terminal sessions return through public `show_session`, replay
`SESSION_REHYDRATED` through `watch_events`, preserve the daemon-local session
token, and remain closeable through `end_session`. This still is not full
crash/restart product recovery: media/input transports are intentionally
degraded after process restart, and the live verifier still needs evidence for
media reattachment, rendered frames, endpoint cleanup, and cross-process
restart behavior.
The latest local run,
`target/e2e/frontend-remoteapp-product-flow/20260822-044248-69775/report.md`,
passed the bounded single-machine product-flow bundle after the local Hub was
restarted with the paired `localhost` realm and the device connection-state
projector preserved `hub_api_endpoint` across the
`FRONTEND_CONNECTED` projection.
Current 2026-08-23 local product-flow evidence,
`target/e2e/frontend-remoteapp-product-flow/20260823-both-current-69931/report.md`,
passed with `target_kind=both` on current HEAD. The report covers Hub API
readiness, daemon control/invocation readiness, frontend typecheck,
`DeviceMediaAccess` RemoteApp UI flow, host permission-subject preflight,
target picker freshness, decoded-frame WebRTC for both window and application
targets, and view-only input safety for both window and application targets.
Application decoded-frame evidence showed `capture_scope=AppSurface`,
`display_fallback_used=false`, rendered frames, selected Resource URA session
subjects, and `host_audio_not_implemented` / host-only route non-claims.
Application view-only input evidence preserved pointer/key
`input_scope_unsupported` telemetry over public `remote_desktop.attach`
InvokeBidi.
Latest 2026-08-23 local Hub API readiness attempt,
`target/e2e/hub-api-readiness/20260823-rich-failure-check-70909/report.md`,
failed before RemoteApp product-flow execution:
`runtime_status=projection_present_process_missing`,
`connection_state=START_FAILED_CREDENTIAL_VERIFY`,
`connection_failure.stage=T06_VERIFY_CREDENTIAL`,
`hub_endpoint=https://127.0.0.1:50443`, and `hub_api_endpoint=null`. That is
current environment evidence against product completion; it does not prove
Browser/Tauri lifecycle, cross-device remote target inventory, real OS
app/window capture, input injection, host audio, or network fallback readiness.
The latest full product-flow attempt,
`target/e2e/frontend-remoteapp-product-flow/20260823-live-preflight-82429/report.md`,
failed at the first `hub-api-readiness-preflight` step and propagated the same
credential-verification diagnostics into the product-flow report. No frontend,
host capture, media, or input product evidence was executed after that failed
upstream gate.
After read-time runtime-status credential hydration, the next local attempts,
`target/e2e/hub-api-readiness/20260823-hydrated-health-report-21626/report.md`
and
`target/e2e/frontend-remoteapp-product-flow/20260823-hydrated-health-report-21627/report.md`,
resolved `hub_api_endpoint=http://localhost:8080` and failed on the actual
health probe: `http://localhost:8080/api/v1/health` returned
`connection refused`. Docker was reachable in that preflight. Product-flow still stopped at
the first upstream gate, so no frontend, host capture, media, or input evidence
ran.
The cross-device smoke entrypoint composes the existing two-node EasyRemote CLI
E2E and synthetic media/bidi Docker E2E. Its evidence scope is intentionally
narrow: governed Hub routing, cross-device ability visibility/invocation, and
synthetic stream/bidi carrier receipt chains. It explicitly does not prove real
OS window/application capture, input injection, host audio, NAT/TURN deployment,
or frontend browser rendering.
Historical local cross-device evidence previously failed at the two-node
routing step because the caller's user-scoped `service/alice.pages` owner
projection was rejected by the Hub with `accepted_count=0, expected_count=5`.
That was diagnosed as a Service owner multihost read-model conflict and is now
covered by `service_owner_projection_is_fenced_per_host_device`,
`service_owner_projection_selects_live_host_from_multihost_rows`, and
`handle_advertise_abilities` regression coverage. The latest attempted live
cross-device product smoke still did not produce authoritative product evidence.
Current structured environment reason: `docker info timed out after 3s`.
Cross-device product readiness therefore remains partial.
The cross-device smoke report now records source revision, dirty-state, runtime
image, image id, image creation time, and whether `--build` was requested, so
stale-image failures are not treated as authoritative current-source failures.
They do not prove every operating system, network topology, input mode, codec
path, and frontend lifecycle is product-ready.

## Product closure matrix

| Requirement | Current status | Evidence that exists | Evidence still required before product-complete |
|---|---|---|---|
| Application/window selection and stable capture across macOS/Windows/Linux | Partial | macOS ScreenCaptureKit target model and host decoded-frame harnesses; macOS application capture passes uncommitted same-app same-display windows as `exceptingWindows` so committed window-set sessions do not widen to every same-app window; non-macOS app/window target observation fails closed; frontend session details surface daemon target loss reason/recovery action and the action row can execute `refresh_targets` by refetching target inventory; `check-remoteapp-main-crate-implementation-tests.sh` runs the main-crate app/window target observation and fail-closed implementation tests; device capability projection advertises `production_target_subjects` only when the production gate is ready, keeps `diagnostic_target_subjects` display-only, and exposes `platform_support` for macOS/Linux/Windows with Linux app/window and Windows capture explicitly unsupported; `remoteapp-cross-platform-capture-e2e.sh` verifier defines the live macOS/Windows/Linux capture or explicit unsupported artifact contract | Live `remoteapp-cross-platform-capture-e2e.sh` artifact proving macOS display/window/application capture, Windows/Linux capture or explicit product unsupported state, no display fallback for window/application targets, rendered frames for passing scenarios, selected Resource URA session subjects, and visible terminal receipts |
| Mouse/keyboard input injection is controllable, low-latency, and permission-correct | Incomplete | App/window sessions downgrade to `view_only`; pointer/key frames are policy-gated; clipboard/file-drop are unsupported; session views now expose `input_readiness` with requested/effective mode, interactive readiness, and blocked reason; frontend session details separately surface input scope plus pointer/keyboard enablement, such as `input scope display_global · pointer+keyboard` or `input scope display_global · no controls`; frontend input sending consumes daemon `input_readiness` and fails closed before sending pointer/key frames; frontend rejects missing or stale pointer `target_geometry_revision` before WebRTC data-channel send; frontend now refuses to enqueue RemoteApp input when the RTC data-channel backlog exceeds the explicit input bound and attaches monotonic `client_sequence` plus `sent_at_ms` telemetry to accepted frames; daemon pointer/key input schema accepts frontend `sent_at_ms` and `client_sequence` metadata and preserves `client_sent_at_ms`/`client_sequence` in input applied/rejected events; diagnostic InvokeBidi input responses preserve the same telemetry for probe correlation, including `target_input_not_ready`; host view-only input safety E2E now opens public `remote_desktop.attach` InvokeBidi and requires app/window pointer/key diagnostic frames with `client_sequence`/`client_sent_at_ms` to be rejected as `input_scope_unsupported`; frontend `watch_events` surfaces daemon input activation blocks and `INPUT_FRAME_REJECTED` reasons without closing media transport; display interactive sessions require an explicit input-control consent ticket before resolving `display_global` input scope; target tracker input loss projects `target_input_not_ready`; OS accessibility absence still reports `input_injection_unavailable`; `remote_desktop.request_permission` contract and frontend status expose Accessibility/input-injection permission alongside Screen Recording; device capability projection exposes `input_control_support` with macOS display tied to runtime permission and macOS window/application plus Linux/Windows input unsupported; the UI offers `Request permission` from daemon input-injection blockers; `check-remoteapp-main-crate-implementation-tests.sh` runs the main-crate current-session input policy implementation test; `remoteapp-input-injection-e2e.sh` verifier defines the live pointer/keyboard injection artifact contract | Live `remoteapp-input-injection-e2e.sh` artifact proving focus validation, coordinate mapping, target epoch/revision checks on execution path, OS input permission, input-control consent, display_global input scope, bounded latency measurements, observed OS pointer/key effects, and visible terminal receipt |
| Audio/video codec, frame rate, bitrate adaptation, and drop policy are product-ready | Partial | macOS H.264/WebRTC path, VideoToolbox descriptor, adaptive bitrate helper, queue/drop boundary tests, session/device capability view explicitly reports `host_audio_not_implemented`, device capability projection exposes `media_pipeline_support` with video-only scope, H.264 payload metadata, bounded queue stale-frame drop policy, native adaptation policy, diagnostic stale-frame policy, `host_audio_not_implemented`, and missing media-adaptation E2E as a product blocker, frontend session details surface `host_audio_not_implemented` from daemon audio readiness projection, frontend session details surface media bitrate/FPS/drop/backpressure stats, ABI v8 raw stream metadata contract preserves canonical lifecycle fields without base64 payload projection, release packaging/install gates ship the v8 raw-stream export allowlist beside the base v7 ABI contract, `check-remoteapp-main-crate-implementation-tests.sh` runs the main-crate WebRTC app/window display-fallback rejection and native plugin catalogue implementation tests, and `remoteapp-media-adaptation-e2e.sh` verifier defines the live audio/video adaptation artifact contract | Live `remoteapp-media-adaptation-e2e.sh` artifact proving negotiated video codec, payload content type, transport, requested/effective/measured FPS, target and observed bitrate, keyframe cadence, bounded frame latency, real host audio, baseline/degraded-network/backpressure scenarios, bounded queue depth, explicit stale-frame drop policy, adaptation/drop evidence under impairment, rendered media after adaptation, selected Resource URA session subjects, and visible terminal receipts |
| Multi-window/multi-application independent tracking works as an execution effect | Partial | Target tracker state machine, move/resize/loss/rebind events, same-display application window-set rebind, ScreenCaptureKit `exceptingWindows` for uncommitted same-app windows; frontend session details surface target tracking recovery state and a Refresh targets CTA instead of opaque failure; `check-remoteapp-main-crate-implementation-tests.sh` runs main-crate app/window target rebind implementation tests; `remoteapp-multi-window-tracking-e2e.sh` verifier defines the live independent tracking artifact contract | Live `remoteapp-multi-window-tracking-e2e.sh` artifact proving independent concurrent window streams, distinct Resource URAs/session ids/stream ids/media source epochs/frame source ids, non-interleaved frames, move/resize geometry revisions, application window-set churn with pending media rebind and `TARGET_REBOUND`, target loss with bounded rebind or explicit rebind failure, multi-display `MultiAppSurface` pass or explicit product unsupported state without capture start, selected Resource URA session subjects, and visible terminal receipts |
| Disconnect/reconnect, session resume, consent revoke, cancel, timeout are complete | Partial | Lease monitor, refresh/end session, explicit `end_session` and lease-timeout `terminal_receipt` projection, host `host-remoteapp-session-timeout-e2e.sh` entrypoint creates a short-lived session through the CLI, observes `session_expired` through public `show_session`, and verifies post-timeout `end_session` idempotency preserves the original terminal receipt; host `host-remoteapp-session-cancel-e2e.sh` entrypoint creates a live-target session through the CLI, invokes public `remote_desktop.end_session` with `user_cancelled`, observes the closed state through public `show_session`, and verifies repeated `end_session` preserves the original cancel terminal receipt; host `host-remoteapp-permission-revoke-e2e.sh` entrypoint creates a live-target session and waits for real platform permission revoke evidence before accepting public `show_session` projection of `target_permission_revoked`, revoked consent, ordered `TARGET_PERMISSION_REVOKED`/`MEDIA_SOURCE_LOST`/`SESSION_CLOSED` events, and terminal receipt binding; host `host-remoteapp-session-resume-e2e.sh` entrypoint creates a short-lease session, invokes public `remote_desktop.refresh_lease`, waits past the original lease, validates the same non-terminal session through public `show_session`, and then closes it with `resume_e2e_cleanup`; frontend retention of closed `end_session` views with terminal receipt and cleared session token, permission revocation terminates the daemon session with a `terminal_receipt` and frontend terminal sync, target loss and transport failure taxonomy, frontend watch_events recovery handling for degraded and permission-revoked sessions, frontend exposes `Retry session` for daemon retry-session guidance, terminal receipt retention no longer blocks new `create_session`, frontend preserves non-terminal RemoteApp sessions across device-offline presence drops and rebinds through `show_session` + WebRTC restart, canonical SDK cancel/timeout semantics, daemon-local recovery snapshots persist session token/target/consent/event facts, plugin startup rehydrates snapshots into degraded session rows, and runtime coverage proves public `show_session`, `watch_events`, and `end_session` operate on a rehydrated row | Long-outage and network transport reconnect handoff E2E, browser/WebRTC rebind E2E after daemon-session lease refresh, live consent revoke termination E2E pass report from a real host permission revoke, canonical Axon cancel/timeout receipt-chain evidence for RemoteApp abilities, live `remoteapp-crash-restart-recovery-e2e.sh` artifact proving daemon/plugin restart recovery, watch/media reattachment, rendered frames after restart, recovered replay/idempotency/lock guards, original terminal receipt replay, stale socket cleanup, endpoint readiness, and visible terminal receipts |
| NAT/relay/WebRTC/direct fallback network paths are verified | Partial | Typed host/STUN/TURN/EasyNet relay route evidence, source-level provider gates, daemon `client_ice_servers` projection, frontend `RTCPeerConnection` consumption, frontend route-state visibility such as `route host_only · no NAT/relay`, and `remoteapp-network-fallback-e2e.sh` verifier for real network fallback artifacts | Live `remoteapp-network-fallback-e2e.sh` artifact proving real direct, STUN srflx, TURN relay, and EasyNet relay paths with selected candidate-pair evidence, rendered media, redacted credentials, selected Resource URA session subjects, and visible terminal receipts |
| Frontend UI can discover, authorize, start, display, control, and end session | Partial | Frontend subject boundary, dedicated surface gates, component coverage for picker → permission_status preflight → consent → create → WebRTC attach → watch_events → end, denied permission_status remains in the picker with Request permission recovery instead of becoming a session error, target-scoped WebRTC lifecycle unit coverage, watch_events recovery-state coverage, daemon `input_readiness`, executable target refresh recovery, executable retry-session recovery, `media_pipeline_support` session detail projection with video-only/H.264/drop-policy/product-blocker visibility, and `terminal_receipt` session detail projection coverage, permission-revoked terminal sync coverage, product-flow harness entrypoint for combined frontend/host evidence, and `frontend-remoteapp-browser-lifecycle-e2e.sh` verifier for real Browser/Tauri lifecycle artifacts | Live Browser/Tauri E2E artifact with real backend/runtime proving picker → permission → consent → create_session → WebRTC attach → watch_events recovery → media presentation → input/control or policy-block → end_session → visible terminal receipt |
| Cross-device E2E smoke/regression exists beyond local provider boundary | Partial | `remoteapp-cross-device-product-smoke.sh` composes Docker two-node routing and synthetic media/bidi carrier gates; host-local decoded-frame scripts cover local capture/render decode | RemoteApp-specific two-device or equivalent network namespace E2E with remote target inventory, remote WebRTC/media from actual display/window/application capture, input policy, and teardown evidence |

Latest bounded lifecycle evidence on 2026-08-23: local macOS window and
application live runs passed through public catalog-resolved lifecycle Ability
URAs and session approval receipt causal context:
`target/e2e/host-remoteapp-session-timeout/20260823-live-window-causal-222646-11519/report.md`,
`target/e2e/host-remoteapp-session-cancel/20260823-live-window-causal-222700-12564/report.md`,
`target/e2e/host-remoteapp-session-resume/20260823-live-window-stable-222830-19233/report.md`,
`target/e2e/host-remoteapp-session-timeout/20260823-live-application-causal-222846-20408/report.md`,
`target/e2e/host-remoteapp-session-cancel/20260823-live-application-causal-222859-21255/report.md`,
and
`target/e2e/host-remoteapp-session-resume/20260823-live-application-stable-222859-21261/report.md`.
This evidence is host-local lifecycle proof only; it does not prove real
permission revoke, long-outage reconnect, browser/WebRTC rebind, crash/restart
recovery, cross-device transport, or cross-platform OS support.

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
