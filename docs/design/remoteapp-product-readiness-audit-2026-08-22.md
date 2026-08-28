# RemoteApp Product Readiness Audit — 2026-08-22

Status: product closure incomplete.

2026-08-28 active-media isolation delta: the plugin-private protocol crate now
contains `remoteapp_media_session_v1`, a binary H264/Opus data-plane contract
for one immutable session/transport/media-source process generation. It binds a
daemon nonce and exact target digest, separates control/video/audio lanes, caps
allocation before payload reads, and supplies a stateful ingress validator for
prepared/activated/terminal lifecycle, per-lane monotonic sequence, stale
generation rejection, codec-generation IDR/SPS/PPS restart, and the exact Opus
packet ceiling. The Unix active media path now instantiates that contract in a
generation-scoped `easynet-remoteapp-media-host`: macOS capture/VideoToolbox/
Opus and Linux X11/OpenH264 execute outside the daemon, publish encoded frames
through bounded shared mappings plus 56-byte notifications, and retain the
mapped payload as the WebRTC `Bytes` owner. A real Linux/X11 two-window process
test covers exact window/application capture, H.264, reconfiguration and typed
membership invalidation. Windows hosted source dispatch and shared mappings now
cross-compile, but real Windows and signed cross-device A/V evidence remain
incomplete, so this is not product-completion evidence.

2026-08-28 audit delta: Browser `presenting` is no longer accepted as decoded
media proof. Readiness requires fresh daemon-admitted evidence for the exact
session/transport/binding/media-source/pipeline/codec tuple, and the authored
and compiled report schema are parity-tested. Host-audio offer admission now
uses a plugin-owned fixed-state coordinator with a capacity-one wake channel,
source-scoped synchronous invalidation and a supervisor that remains joinable
when one native probe attempt blocks. These changes close false-readiness and
unbounded-queue seams; they are not live cross-platform host-audio or full
RemoteApp product-completion evidence.

This audit separates verified targeted-session architecture from full
interactive RemoteApp product readiness. Passing the current boundary gates
does not mean RemoteApp is product-complete.

The machine-readable gate source for this audit is
`docs/design/remoteapp-product-readiness-matrix.json`. The Markdown table below
is explanatory; the JSON matrix is the product-closure status contract consumed
by `tools/scripts/check-remoteapp-product-closure-audit.sh`.

The current checked-in product-completion reports remain `mode=self-test`,
`evidence_origin=contract_self_test`, and `product_complete_claim=false`; any
nested `live_runner` values inside those reports are synthetic verifier inputs.
Current focused live child proofs outside the checkout under `/tmp` include a
complete local Browser window lifecycle plus reproducible constrained direct
and TURN-relay-only network scenarios. They materially advance those focused
rows, but they are neither a checked-in same-campaign bundle nor a
product-completion decision. Historical
2026-08-23 live paths retained below are not present in the current checkout
and cannot be used as reproducible current product-completion evidence.
The aggregate production gate rejects an explicit `contract_fixture` even if
its origin string is rewritten to `live_runner`; contract-fixture validation
mode always emits `product_complete_claim=false`. Production `--check` now also
requires one DSSE/Ed25519-signed campaign bundle rooted in an external role
trust bundle. Every domain attestation must bind the same campaign/run, clean
Git revision, runtime/plugin/frontend build digests, validity window, selected
Resource, session, immutable descriptor ref, Invocation URAs, Axon-verified
admission/terminal receipt hashes, and recomputed report/artifact digests. A
passing aggregation produces only an eligible candidate with
`product_complete_claim=false`. A separate product-completion authority must
sign the exact candidate before the finalizer atomically consumes its campaign
in the durable replay ledger and emits a claim. No such signed live campaign or
completion decision exists in the current checkout, so this stronger boundary
still produces no product-complete claim; `evidence_origin` alone is only a
diagnostic label.

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
- `tools/scripts/remoteapp-product-completion-e2e.sh`
- `tools/scripts/remoteapp-product-finalize.py`
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
macOS window/application targets may project `target_local` only when the grant
contains input control and every pointer/key event passes a fresh host target
guard. Without that grant they remain `view_only` with
`input_consent_required`; non-macOS target-local input remains unsupported.
The fresh guard shares the target monitor's plugin-owned single-flight native
snapshot executor and has a 50 ms monotonic input deadline. A stuck host window
provider therefore rejects the frame as `target_input_guard_deadline_exceeded`
without spawning replacement native calls or indefinitely blocking the input
channel.
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
production subjects, while the xcap/OpenH264 baseline can execute exact
display/window/application capture without being promoted to production-ready.
That projection uses the same runtime native backend descriptor as
`production_gate_view`, so Screen Recording permission denial and non-macOS
`not_installed` state are not hidden by the compile-time macOS descriptor.
The lifecycle/input boundary gate and mutation tests now pin this projection
shape, including the runtime-native descriptor source, production-ready gate,
target-scoped diagnostic subjects, blocked reason, and production subject source.
The same device capability metadata now exposes `metadata.platform_support` for
macOS, Linux, and Windows. macOS target rows follow the native production gate;
Linux and Windows rows expose an executable xcap/OpenH264 `baseline_ready`
state for display/window/application, but remain outside
`production_target_subjects` until live platform certification exists.
It also exposes `metadata.input_control_support`: macOS display input follows
the runtime Accessibility/input-injection permission, macOS window/application
input follows the same permission plus the target-local execution guard, and
Linux/Windows input injection has guarded executable baselines: Windows uses
User32 `SendInput` with UIPI fail-closed behavior and Linux uses X11/XTest while
Wayland remains unsupported until a portal session is bound. Live OS-effect
certification is still missing, so neither platform is product-ready.
The target resolver now consumes one typed compiled-guard policy instead of a
macOS-only predicate. Default Windows and Linux native-media builds therefore
make their existing exact-target guard/focus/injection implementations reachable
as `target_local` after explicit consent. A headless Windows/Linux build without
xcap observation remains view-only even if a global injection API is present.
Capability metadata exposes compiled target-guard readiness separately from
runtime injector readiness, preventing the latter from laundering an unsafe
window/application claim. Pure policy and mutation tests cover this reachability;
they are not Windows/Linux host-effect evidence.
The frontend product-flow script is a runnable product-flow harness entrypoint:
with an explicit `--run`, it first verifies Hub API reachability, then product
runtime readiness for daemon control/invocation, then composes frontend
typecheck/UI flow coverage with host permission-subject, target-freshness,
decoded-frame, and view-only input E2E harnesses. A skipped/self-test report
from that entrypoint is only harness evidence, not product completion.
The Browser/Tauri lifecycle verifier,
`tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh`, defines the
required artifact contract for the real bounded Playwright/Chrome runner in
`../EasyNet/Frontend/scripts/remoteapp-browser-lifecycle.mjs`: proof mode
`real_browser_tauri_lifecycle`, `component_mock=false`,
`real_backend_runtime=true`, ordered picker/permission/consent/create/production
`remote_desktop.set_description` WebRTC connection/
watch/media/media-pipeline-support/input/end/terminal-receipt steps, public
RemoteApp ability names, host-local `permission_status` and optional
`request_permission`, selected Resource URA
binding for session abilities, real browser/Tauri automation evidence source
and strictly increasing `observed_at_ms` for every step, rejection of
component-snapshot-only evidence, connected WebRTC state, attached media stream
evidence, visible media element, positive rendered frame count, visible `media_pipeline_support`,
visible input status with either applied-input telemetry or explicit
policy-block reason, and no product-complete claim. Its self-test proves only
the validator. The production browser runner must not substitute diagnostic
`remote_desktop.attach` for the WebRTC signaling path. A current live artifact
now passes that full local window lifecycle; it is described with its residual
scope below and does not certify successful OS input, another platform, or a
remote device.
The frontend session-details surface now also renders daemon route state; a
host-only WebRTC route is visible as `route host_only · no NAT/relay`, so
transport presence is not confused with production NAT/relay readiness.
The same surface now renders a compact media quality summary from daemon/browser
stats, including bitrate, outbound FPS, total drops, and RTP backpressure.
It also renders daemon-projected `media_pipeline_support`, including H.264/Opus
pipeline identity, bounded stale-frame drop policy, audio/video scope when the
native macOS backend is ready, and the remaining live media-adaptation blocker.
The WebRTC client explicitly requests a receive-only audio transceiver, combines
remote audio and video tracks in one presentation stream, and exposes autoplay
recovery. This is implementation evidence, not decoded cross-device host-audio
or degraded-network E2E evidence.
The aggregate product-completion gate additionally binds frontend product-flow
host subreports to the target kind required by each step. Window decoded-frame
and view-only-input steps must be backed by `target_kind=window` host evidence;
application decoded-frame and view-only-input steps must be backed by
`target_kind=application` host evidence. This prevents a real but differently
scoped host run from satisfying the wrong window/application product-flow
requirement.
For every report or product-flow subreport that references a live
`evidence_json` artifact, the aggregate gate also parses that artifact and
requires its own `status=passed`. Domain verifiers still own their detailed
capture/input/media/network/lifecycle contracts, but an empty, failed, invalid,
or stale-looking evidence file cannot be accepted merely because a sibling
summary report says `status=passed`.
The network fallback verifier,
`tools/scripts/remoteapp-network-fallback-e2e.sh`, defines the live artifact
contract for real direct, STUN srflx, TURN relay, and EasyNet relay paths. Its
identity model now matches the production Browser flow instead of turning both
transport peers into Devices: `caller_ura` is the admitted User/Agent/Authority,
`callee_ura` is the device-sponsored Remote Desktop SystemAgent,
`provider_device_ura` is its execution host, and `client_endpoint_id` is an
opaque Browser/network peer correlation id. It requires the production
`create_session`, `set_description`, `watch_events`, `report_client_state`, and
`end_session` sequence; diagnostic InvokeBidi `attach` cannot satisfy the
network proof. The real Browser runner now reads
`RTCPeerConnection.getStats()` before its rendered-frame observation and
records the selected pair's stable pair/local/remote ids, types, route class,
protocol, nomination/state, byte counters, RTT, and redacted Invocation tuple
identity. The verifier additionally requires applied allowed/blocked fixture
constraints, media bound to the same Resource/session/route/pair after pair
observation, credential redaction, and a visible terminal receipt for every
scenario.
`tools/scripts/host-remoteapp-turn-relay-e2e.sh` now provides a reproducible
focused TURN child runner. It builds a content-addressed coturn fixture, starts
the paired daemon with a temporary relay configuration, constrains the Browser
to relay-only ICE, requires server-observed allocations in addition to Browser
RTCStats/relay SDP, validates the child artifact, and restores the ordinary
daemon. This closes one real TURN scenario; it does not synthesize the three
missing route classes.
`tools/scripts/host-remoteapp-direct-e2e.sh` provides the corresponding focused
direct-route runner. It removes every daemon STUN/TURN/EasyNet relay variable,
records that constraint before daemon boot, requires zero projected client ICE
URLs and host-only local/remote SDP, then accepts only a selected direct host
pair with later media and terminal cleanup. Its 2026-08-26 live macOS/window
run passed session `rdp-6672a930781800a72ab45782` with a connected, nominated,
succeeded UDP host/host pair, positive bidirectional bytes, three later Browser
frames, and a `caller_ended` receipt; the ordinary daemon was restored at J800.
The native WebRTC media stats projection now carries the selected pair's
`local_candidate_type`, `remote_candidate_type`, `selected_route_class`, and
`protocol`, while keeping candidate addresses and credentials out of the product
stats payload. `selected_route_class` distinguishes direct host-only,
STUN/reflexive, and relay-selected pairs without inferring TURN versus EasyNet
relay subtype.
Its self-test proves only the artifact validator. The live TURN child is real
relay reachability evidence, and the constrained direct child also now passes.
`tools/scripts/host-remoteapp-stun-srflx-e2e.sh` now makes the remaining STUN
child explicit and fail-closed. The initial pinned-coturn/Docker Desktop
topology could observe Binding traffic but could not expose the Browser VM NAT
mapping back to the provider, so it was not a valid reflexive-only return path.
The runner now rejects that macOS context, runs a bounded RFC 5389 Binding-only
observer on the provider host, and requires an externally reachable VM-NAT
Browser context. It still projects only a redacted `stun:` URL through the real
daemon, admits only `srflx`/`prflx` Browser outbound while retaining
`host`/`srflx`/`prflx` provider inbound return candidates, records directional
accepted/rejected counters, requires server Binding observation before pair
selection, enforces an outer Browser deadline, validates later media and
terminal cleanup, and restores the ordinary daemon. This prevents host/host
direct selection without requiring the provider to advertise a redundant
srflx candidate. Outbound admission covers both trickled candidates and
candidates embedded in the Browser's initial local SDP; evidence fails if the
Browser-local selected candidate is not reflexive or if the projected offer
still contains a host candidate. The native observer passed
an independent coturn `turnutils_stunclient` interoperability probe from the
VM and returned a real VM-NAT reflexive mapping. Focused positive and mutation
gates pass for this contract. No complete Browser child has passed yet: the
temporary VM context was removed after the topology proof, and the exact active
daemon still reports macOS Screen Recording permission denied. Those facts are
not route evidence. STUN srflx therefore remains open. The static
`EASYNET_REMOTE_DESKTOP_EASYNET_RELAY_*` alias has now
been removed: the Hub implements a credential-authenticated,
session/resource-bound ephemeral relay lease aggregate, and the daemon injects
its client through a Remote Desktop plugin port. Lease refresh and
post-terminal release have unit lifecycle proof, but there is still no real Hub
+ coturn + Browser selected-pair/media proof in the older evidence set.
`tools/scripts/host-remoteapp-easynet-relay-e2e.sh` now encodes that missing
live gate: it enables the Hub lease issuer, runs coturn with the matching TURN
REST secret, keeps static daemon TURN credentials unset, forces the Browser to
relay-only ICE, requires a server-observed allocation and session-bound redacted
Hub lease evidence, and restores ordinary Hub/daemon configuration. The
2026-08-26 `--run` artifact at
`/tmp/remoteapp-easynet-relay-live-20260826-v5/report.json` now passes. The Hub
issued one session/resource-bound five-minute lease, coturn independently
observed three allocations, the Browser selected a nominated/succeeded relay
pair, three frames rendered after selected-pair observation, and
`caller_ended` produced the visible terminal receipt, after which a same-binding
reacquire received HTTP 409 from the Hub release tombstone. Public evidence exposes
`ephemeral_auth_configured` but no username, credential or shared secret, and
the runner restored ordinary Hub and daemon configuration afterward. This
closes the focused EasyNet relay child, not the aggregate four-route network
matrix.

Relay-refresh ownership update (2026-08-27): the Hub refresh aggregate replaces
the prior lease ID rather than creating two independently releasable leases.
The Hub contract test now proves a superseded ID cannot release the refreshed
binding and that the current refreshed ID terminalizes it. The daemon session
transition keeps the fresh lease session-owned on successful rotation, returns
an unattached fresh lease to the runtime when a concurrent terminal/removal
transition wins, and releases that exact current Hub lease. If refresh keeps
failing through expiry, the daemon retires and releases the exact current lease
instead of silently dropping authorization state. A concurrent idempotent
refresh that receives the already-installed lease ID is classified as
`AlreadyOwned` and cannot release the session-owned authorization. Rust tests
cover successful rotation/terminal release, duplicate refresh, and the
lost-owner race. The complete Remote Desktop Rust slice passed 522/522 tests on
2026-08-27 after these changes. This does not replace a live long-session
credential-rotation and post-refresh transport-replacement proof.

The live gate for that remaining seam is now executable rather than implied by
unit coverage. `host-remoteapp-easynet-relay-e2e.sh --refresh-resume` shortens
the real Hub lease TTL and keeps the daemon alive until periodic public
`show_session` observation sees the watchdog-issued distinct lease. Only then
does a bounded coordination signal permit the runner to stop and restart the
paired daemon, while the Browser lifecycle runner requires the same
public session, a distinct refreshed lease ID, a newer transport epoch, a new
PeerConnection, `watch_events` reattachment, decoded media, preserved input
authority, terminal settlement, and Hub tombstone rejection. The Browser stores
only an allowlisted redacted lease projection; the dedicated verifier rejects
cross-session/Resource, unchanged-lease, stale-epoch, missing-media,
missing-terminal, and credential-bearing artifacts. The refresh proof is
projected separately from ordinary EasyNet relay route coverage, and skipped or
self-test reports cannot claim either live coverage. Contract and mutation
tests pass, but no `--run --refresh-resume` artifact has been produced yet, so
the readiness status remains Partial.

A preceding repeated run at
`/tmp/remoteapp-easynet-relay-live-20260826-v4/browser/evidence.json` obtained a
valid Hub lease but terminated between `create_session` and `set_description`
with `target_permission_revoked`; the next unchanged v5 run passed. That failed
attempt is not relay evidence and does not invalidate v5. Its root cause was a
single transient negative host permission preflight being interpreted as an
irreversible revocation. The implementation now has a durable
`permission_verification_pending` state: the first negative sample immediately
pauses input/media without revoking consent or terminating the session, a
positive snapshot restores the same session through a newer transport epoch,
and only a second negative sample confirms `target_permission_revoked` and
enters the persistence-first terminal path. Rust aggregate/recovery tests,
frontend same-session/watch-preservation tests, and static mutation gates cover
the state machine. After rebuilding and restarting the daemon from the current
checkout, three consecutive unchanged macOS EasyNet-relay Browser runs passed:
`/tmp/remoteapp-easynet-relay-live-20260826-v6/report.json`,
`/tmp/remoteapp-easynet-relay-live-20260826-v7/report.json`, and
`/tmp/remoteapp-easynet-relay-live-20260826-v8/report.json`. Each selected a
nominated/succeeded relay-only ICE pair, rendered two or three frames after selection,
published a visible `caller_ended` receipt, rejected terminal lease reacquire,
and restored ordinary Hub/daemon configuration. This closes the observed
transient-permission reliability seam; a deliberate real OS permission revoke
campaign remains separate evidence for the confirmed-revocation branch.
The cross-platform capture verifier,
`tools/scripts/remoteapp-cross-platform-capture-e2e.sh`, defines the live
artifact contract for macOS, Windows, and Linux display/window/application
capture. macOS must pass all three target kinds with rendered frames, exact
target binding, public RemoteApp session abilities, selected Resource URA
subject binding, target identity/frame-source/geometry-revision evidence,
decoded-frame probe binding to that same target identity, selected sentinel
id/hash rendering, and visible terminal receipts. Window/application capture
must also prove unrelated sentinel content did not render in the decoded-frame
probe. Windows/Linux must either pass the same capture scenarios or expose
explicit product unsupported state without creating a capture session or
starting display fallback.
The input injection verifier,
`tools/scripts/remoteapp-input-injection-e2e.sh`, defines the live artifact
contract for focus-safe pointer/keyboard control. It requires real OS input
permission, `input_control` consent, target-kind-derived `display_global` or
`target_local` input scope, target
geometry revision binding, focus validation, coordinate mapping validation,
`INPUT_FRAME_APPLIED` events with `client_sequence` and `client_sent_at_ms`,
strictly increasing applied input sequence, `host_received_at_ms` /
`host_applied_at_ms` timing, stale `client_sequence` rejection, bounded
host-applied latency, stable `input_event_id` identity, target focus epoch
binding, and observed OS input effect from a platform observer that is
independent from the injection path. The OS effect must bind the same
Resource/session/input event/geometry revision/focus epoch after host
application. Window/application artifacts must additionally prove a fresh
identity/visibility/focus/geometry/window-set guard after host receipt and
before every OS apply; visible terminal receipts remain required.
Its self-test proves only the contract validator; it is not successful OS
input-injection evidence.
The media adaptation verifier,
`tools/scripts/remoteapp-media-adaptation-e2e.sh`, defines the live artifact
contract for the RemoteApp audio/video data plane. It requires baseline,
degraded-network, and backpressure scenarios with negotiated video codec,
payload content type, transport, requested/effective/measured FPS, target and
observed bitrate, keyframe cadence, bounded frame latency, real host audio,
bounded queue depth, explicit stale-frame drop policy, bitrate/FPS adaptation
or frame-drop evidence under impairment, adaptation events bound to the same
selected Resource URA, session id, and `media_pipeline_id`, event timestamps
after `impairment_applied_at_ms`, cross-scenario proof that
degraded-network target/observed bitrate drops below baseline and backpressure
frame drops exceed baseline, and cross-scenario comparability over the same
selected Resource URA, `media_pipeline_id`, video codec/transport, and audio
codec. It also requires decoded media render-probe evidence bound to the same
selected Resource URA, session id, `media_pipeline_id`, video codec, video
transport, and audio codec, with decoded video/audio counts, video/audio
payload hashes, and post-adaptation observation for degraded-network and
backpressure scenarios. Public RemoteApp session abilities, selected Resource
URA subject binding, and visible terminal receipts remain required. Its self-test proves only the contract validator; it is not codec,
host-audio, soak, or degraded-network product evidence.
The executable host runner,
`tools/scripts/host-remoteapp-media-adaptation-e2e.sh`, closes the orchestration
gap between that verifier and the real EasyNet Browser lifecycle. It runs
baseline, degraded-network, and backpressure sessions sequentially, requires
explicit apply/reset commands, reapplies reset from an exit trap after Browser
failure, retains raw per-scenario artifacts, records only redacted command
fingerprints, and delegates fact projection to the canonical aggregator. This
runner makes the live matrix directly executable; it is not itself a passed
host artifact.
The native ScreenCaptureKit stream now captures screen frames and system audio
together. VideoToolbox emits H.264 while a bounded audio queue converts 48 kHz
stereo float PCM into 20 ms Opus frames; both tracks use the same WebRTC
PeerConnection. The media loop emits
`remoteapp_media_pipeline_stats_v1` in `MEDIA_PIPELINE_STATS` rows with selected
Resource URA, session id, transport epoch, media source epoch, stable media
pipeline id, negotiated H.264/Opus payload metadata, WebRTC transport,
requested/effective/measured FPS, target/observed bitrate, bounded stale-frame
drop policy, audio/video backpressure counters, audio pipeline readiness, a
separate `audio_media_observed` signal, and adaptation events. Silence may leave
`audio_media_observed=false` without making the negotiated pipeline unhealthy;
product completion still requires positive decoded audio evidence in a live
media-adaptation artifact.
Windows and Linux host audio are deliberately not advertised. Their canonical
media-host session adapters do not yet emit validator-checked Opus, so device
capability projection and SDP admission return
`active_media_session_audio_unavailable` before answer commit. Earlier
daemon-local WASAPI/PipeWire primitives and source-plan tests are not product
evidence and are not selected by production WebRTC. Product readiness requires
implementing the equivalent hosted Opus adapter and proving selected output is
present while unrelated application audio is absent on a second device.
The capability view reports missing media-adaptation E2E as a product blocker;
the required artifact must contain live decoded audio/video evidence.
The multi-window tracking verifier,
`tools/scripts/remoteapp-multi-window-tracking-e2e.sh`, defines the live
artifact contract for independent app/window tracking as an execution effect.
It requires independent concurrent window streams with distinct Resource URAs,
session ids, stream ids, media source epochs, and frame source ids;
non-interleaved frames; per-stream decoded-frame probes bound to each selected
Resource URA, session id, stream id, frame source id, media source epoch, and
selected sentinel id/hash; per-stream selected target sentinel rendering; no
foreign or cross-stream sentinel leakage in either stream summaries or decoded
frame probes; ordered `TARGET_MOVED`/
`TARGET_RESIZED` geometry churn with increasing geometry revisions;
cross-display application window-set churn with pending media rebind,
`TARGET_REBOUND`, committed window-set sentinel rendering after rebind, and
uncommitted same-app window sentinel absence; target loss with bounded rebind
or explicit rebind failure; multi-display application pass through
decoded multi-display `MultiAppSurface` evidence without display fallback;
public RemoteApp session abilities; selected Resource URA subject binding; and
visible terminal receipts. Its self-test proves only the contract validator;
it is not real multi-window tracking evidence.
The crash/restart recovery verifier,
`tools/scripts/remoteapp-crash-restart-recovery-e2e.sh`, defines the live
artifact contract for deterministic RemoteApp recovery after daemon, plugin
worker, terminal-receipt, and stale-socket interruptions. It requires public
RemoteApp abilities, same-session recovery after daemon restart, recovered WAL/
idempotency/replay-guard/lock state, ordered lifecycle events bound to the
selected Resource URA and session id, `show_session` validation, watch-events
and media reattachment, first rendered frame after media reattachment, plugin
worker/target monitor restart without minting a new public session, original
terminal receipt replay after crash during close, public `show_session` after
terminal receipt replay, explicit stale control/invocation socket cleanup,
endpoint readiness after daemon-ready observation, selected Resource URA
subject binding, and visible terminal receipts. Its self-test proves only the
contract validator; it is not real crash/restart recovery evidence.
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
token, and remain closeable through `end_session`. Batch startup recovery now
reports and skips corrupt snapshot rows without dropping valid rows, and Unix
recovery state is private because snapshots include the daemon-local session
token. Rehydrated non-terminal sessions can now leave the `Suspended` phase and
start a fresh media negotiation epoch using the same session id. Rehydrated
non-terminal sessions also re-register with the plugin target monitor, and the
target monitor now keeps desired tracking state outside the worker thread so
worker replacement can seed tracking from plugin state. Recovery snapshots
whose lease elapsed while the daemon was down are now settled synchronously at
startup as durable `session_expired` terminal rows instead of being exposed as
recoverable degraded sessions. This still is not full crash/restart product
recovery. A 2026-08-26 real Browser run now proves the orderly daemon
stop/start path: the same non-terminal window session was rehydrated through
public `show_session`, the old PeerConnection retired, Runtime transport epoch
increased from `1787686710123091` to `1787686896984117`, a new PeerConnection
and `watch_events` subscription connected, and a `1688x1080` frame rendered
after resume before the same session ended with a visible `caller_ended`
receipt. Input authority remained stable but macOS Accessibility was
policy-blocked before and after restart. This does not prove an ungraceful
process kill by itself. The reusable
`tools/scripts/host-remoteapp-daemon-sigkill-e2e.sh` then killed exact,
path-verified `target/debug/easynet-daemon` PID `4365` with `SIGKILL` during
active window session `rdp-b8606004250fbe8de38e6340`, restarted path-verified
PID `53848` to `J800`, and passed its canonical Browser verifier. Device
presence moved `J700 -> C440 -> J700`; the same session rehydrated; a replacement
PeerConnection with a newer Runtime transport epoch connected; `watch_events`
reattached; a frame rendered after recovery; and the recovered session
ended with a visible `caller_ended` receipt. The live report is
`/tmp/remoteapp-daemon-sigkill-runner-live-20260826-v3/report.md`. The same run
proved Unix stale-socket cleanup rather than inferring it from successful boot:
control inode `282498224` and Invocation inode `282498231` remained present but
unreachable after `SIGKILL`; daemon listener bind replaced them without manual
or wrapper-side cleanup with reachable inodes `282510760` and `282510765`.
This closes the active-session daemon-process-kill scenario and its Unix
stale-socket child row. A separate 2026-08-26 live close-crash campaign then
used `tools/scripts/host-remoteapp-terminal-receipt-crash-e2e.sh` to kill
feature-built daemon PID `91393` after authoritative terminal snapshot
promotion but before the Browser received the `end_session` response. Device
presence moved through `C440` and restarted PID `92151` reached `J800`; the
Browser recovered the same session `rdp-0a2fa34448b53eb7dfc35ed4` through
public `show_session`. The private crash marker, authoritative recovery
snapshot, paired-user signed public `show_session`, and repeated public
`end_session` all contained one byte-for-byte equal `caller_ended` terminal
receipt, and repeated end reported `already_ended=true`. The live report is
`/tmp/remoteapp-terminal-receipt-crash-live-20260826-v4/report.md`. Product
builds contain no callable crash surface: the exact promotion fault point is
compiled only with `remoteapp-e2e-fault-injection` and requires an owner-only,
one-shot, exact-session arm file. This closes crash-during-close receipt replay.
A third 2026-08-26 live campaign used
`tools/scripts/host-remoteapp-target-monitor-worker-recovery-e2e.sh` to crash
only target-monitor generation `1` for Browser session
`rdp-66843994a4396d038cc76b94`. Feature daemon PID `62280` remained the exact
J800 process before and after the fault. Browser public events and the durable
recovery snapshot both preserved the ordered
`PLUGIN_WORKER_CRASHED -> PLUGIN_WORKER_RESTARTED -> TARGET_MONITOR_RESTARTED`
sequence and bound replacement generation `2` to the private exact-session
fault marker. The public session, selected window Resource, consent epoch,
target binding epoch, WebRTC transport epoch, and media-source epoch remained
unchanged; the Browser presented a later frame without new consent and the
same session ended with a visible `caller_ended` receipt. The live report is
`/tmp/remoteapp-target-monitor-worker-live-20260826-v4/report.md`. Ordinary
builds contain no worker-crash surface because the owner-only one-shot arm is
also compiled only with `remoteapp-e2e-fault-injection`. This closes the
macOS/window target-monitor worker-only child scenario. Windows named-pipe
restart, cross-device recovery, and the remaining aggregate crash matrix stay
open, so recovery is still partial.
Diagnostic InvokeBidi preview now has generation-scoped transport-manager
ownership and a single task-group completion receipt covering control,
forwarding, capture/encoding, and the blocking H.264 worker. Explicit close,
lease timeout, and host permission revocation remain in durable `Closing`
until that receipt arrives; a disconnected completion channel fails closed.
Unit coverage proves stale preview activation cannot replace a newer
generation and that terminal publication follows task-group settlement. Bidi
terminal-frame publication is now deadline-bounded (and nonblocking from the
blocking encoder thread), so an abandoned or slow client queue cannot prevent
the task-group completion receipt and resource teardown. This removes the
previous stop-only/backpressure lifecycle seam, but it is not live product
evidence. Direct WebRTC and preview settlement now share one bounded deadline,
so a hung platform media worker cannot hang the caller or plugin shutdown; a
miss leaves the durable row in `Closing`. Both transports, pending endpoint
setup reservations, and partially-created peer cleanup now transfer to one
process-owned round-robin settlement executor. It retains completion receivers
and worker join handles across deadline misses, fences endpoint generations,
keeps a drop-first unknown setup reservation visible to a later terminal sweep,
and transfers explicit failures or a panicking cleanup job into typed
quarantine without dropping its ownership even after manager submission
handles disappear. Quarantine records job/session/failure/projection state,
retries durable outcome projection with bounded backoff, surfaces unhealthy
transport settlement health in `show_session`, and closes new-session
admission while an unproven resource owner remains. Affected sessions publish
a durable terminal `Failed` receipt instead of remaining anonymously in
`Closing`; the retained owner still requires controlled runtime recycle before
admission reopens. Peer callbacks retain the manager
weakly and the media worker retains only an independent Tokio runtime handle,
removing the manager/peer/worker reference cycles. Explicit end, expiry, and
permission revocation share one terminal finalizer. That finalizer constructs
the terminal session on a private candidate and writes it to a
non-authoritative staging file outside the session mutex. Under a short
aggregate lock it revalidates the exact event/update/reason revision,
atomically promotes only that matching staged body into the recovery authority,
and publishes the identical in-memory `Closed`. A concurrent final media-stat
update therefore leaves the previous authoritative `Closing` snapshot intact
and forces a fresh stage rather than creating a stale absorbing `Closed`
snapshot. Persistence failure therefore
remains observable as durable `Closing` and is retried by the settlement job
with bounded exponential backoff instead of being misreported as settled or
busy-spinning. Zero-transport end, expiry, and
permission-revocation paths use this same persistence-first finalizer instead
of directly publishing a terminal receipt. These are component lifecycle guarantees, not
live cross-device recovery evidence; cross-device recovery remains open.
The latest local run,
`target/e2e/frontend-remoteapp-product-flow/20260822-044248-69775/report.md`,
passed the bounded single-machine product-flow bundle after the local Hub was
restarted with the paired `localhost` realm and the device connection-state
projector preserved `hub_api_endpoint` across the
`FRONTEND_CONNECTED` projection.
Historical 2026-08-23 local product-flow evidence (artifact not present in the
current checkout),
`target/e2e/frontend-remoteapp-product-flow/20260823-both-current-69931/report.md`,
passed with `target_kind=both` on current HEAD. The report covers Hub API
readiness, daemon control/invocation readiness, frontend typecheck,
`DeviceMediaAccess` RemoteApp UI flow, host permission-subject preflight,
target picker freshness, decoded-frame WebRTC for both window and application
targets, and view-only input safety for both window and application targets.
Application decoded-frame evidence showed `capture_scope=AppSurface`,
`display_fallback_used=false`, rendered frames, selected Resource URA session
subjects and host-only route non-claims. This older artifact predates the
macOS host-audio implementation and therefore is not audio product evidence.
Application view-only input evidence preserved pointer/key
`input_scope_unsupported` telemetry over public `remote_desktop.attach`
InvokeBidi. The daemon input data-channel loop also rejects replayed or
out-of-order frames as `stale_client_sequence` whenever the client supplies
`client_sequence`, so monotonic input telemetry is enforced at the plugin
execution boundary instead of being only observational. Applied input events
now include daemon-side `host_received_at_ms`, `host_applied_at_ms`, and
clock-safe `latency_ms` telemetry when the client supplied
`client_sent_at_ms`, giving the live input-injection verifier host execution
evidence instead of only frontend timestamps. If OS input permission is denied
after input activation, the session now emits `INPUT_PERMISSION_BLOCKED`,
downgrades input lifecycle without closing media, and exposes
`request_permission` recovery rather than leaving the UI to infer permission
loss from repeated frame rejections. The session aggregate also retains the
runtime input blocker and projects it through `show_session`
`input_readiness.blocked_reason` until input is proven active again, so a
refresh or reconnect does not lose the input-only permission state. A later
host-applied input frame now clears the blocker through the session aggregate,
re-enters `InputActive`, and emits `INPUT_PERMISSION_RESTORED` instead of
leaving the frontend to infer recovery. Recovery snapshots persist the same
session-local runtime input blocker and the plugin startup rehydrate regression
verifies public `show_session` still projects it
after restart.
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
It also aggregates child `caller_ura`/`provider_ura` topology into observed
device pairs, records `distinct_device_uras_observed`, and marks
`local_provider_boundary_only`; a nominally completed run now fails when
distinct device URAs were not observed, so same-device/local-provider reports
cannot be mistaken for cross-device product evidence.
They do not prove every operating system, network topology, input mode, codec
path, and frontend lifecycle is product-ready.

The product-completion gate,
`tools/scripts/remoteapp-product-completion-e2e.sh`, is the only aggregate gate
that may turn the individual RemoteApp evidence reports into an eligible
completion candidate. It never emits a product-complete claim. It requires
report JSONs for frontend product-flow, Browser/Tauri
lifecycle, cross-device smoke, cross-platform capture, input injection, media
adaptation, multi-window tracking, network fallback, window/application
session timeout, window/application session cancel, window/application
permission revoke, window/application lease refresh survival, real browser
transport resume, and crash/restart recovery. Browser transport resume is a
separate proof domain: it must preserve the same public session while retiring
the old PeerConnection, accepting a strictly newer daemon-issued transport
epoch, recreating `watch_events`, presenting a decoded frame, and preserving
input authority after reconnect. Missing reports fail closed, child verifiers must not claim product
completion, and cross-device evidence must not be local-provider-only. The gate
also checks the stable `script` identity and expected `target_kind` of each
required lifecycle report, so one target kind cannot stand in for the other.
It also requires cross-platform capture and input-injection platform summaries
to be `passed` for macOS, Windows, and Linux. Explicit `unsupported` platform
states remain valid readiness/blocker evidence in their domain verifiers, but
they cannot satisfy the aggregate product-complete claim. The gate also
requires domain reports to name an existing `evidence_json` artifact, requires
the frontend product-flow report to expose passed Browser/Tauri, cross-device,
permission-subject, separate window/application target-picker freshness,
window/application decoded-frame, and
window/application view-only-input steps with `target_kind=both`, and requires
those product-flow steps to have traceable `result.json` step artifacts plus
subreport/evidence artifacts for Browser/Tauri, cross-device, and host steps.
Those subreports carry stable `script` identity, including the host
permission-subject, both target-picker variants, decoded-frame, and
view-only-input verifiers. Target-picker subreports are pinned to their exact
`target_kind`; the application evidence additionally binds app identity, owner
pid, exact native window membership/order, and window-set epoch.
Cross-device topology must include observed caller/provider device pairs with
distinct device URAs. In addition, the gate consumes only a signed same-run
campaign and refuses unsigned path-selected reports, mixed builds, tampered
artifacts, untrusted signer roles, or expired campaigns. The separate
`remoteapp-product-finalize.py` boundary requires an independently custodied
`product_completion_authority` DSSE decision bound to the exact candidate,
then consumes the campaign replay id and emits the only
`product_complete_claim=true` report. An exact retry can recover publication;
a different completion statement for the consumed campaign is rejected. The
report embeds the exact candidate bytes and can be independently verified
against the completion signature, canonical final projection, and fixed system
replay record. The authority workflow has explicit `prepare` and `assemble`
phases: `prepare` independently pins all 19 product domains and emits canonical
DSSE PAE bytes; an external KMS/HSM signs those bytes; `assemble` verifies the
64-byte Ed25519 signature and completion-only key role before emitting the
envelope. Private signing keys never enter the repository tool. This
gate is not itself new product evidence; it prevents
partial, target-narrowed, or empty-shell evidence from being reported as full
interactive desktop completion.

## Product closure matrix

Input implementation update (2026-08-24): macOS window/application sessions
now resolve `target_local` only with explicit input-control consent. Pointer and
keyboard execution re-enumerates host state immediately before CGEvent posting
and rejects identity, display, visibility, focus, geometry, or application
window-set drift. The CLI exposes this grant as `--input-control`; omission is
still the E2E-11 view-only path. This moves the implementation status from
incomplete to partial, but does not satisfy product closure until live E2E-14
artifacts independently observe selected-target OS effects and bounded latency.

Live-input runner update (2026-08-24):
`host-remoteapp-target-input-e2e.sh` now composes the existing decoded-frame
receiver with the canonical `easynet.remote_desktop.input.v1` WebRTC data
channel, explicit `--input-control` consent, daemon applied/rejected events,
public `watch_events`/`end_session`, and two independent AppKit processes that
record selected and unrelated mouse/key callbacks. The first local window run
reached the real `remote_desktop.create_session` permission gate and failed
closed because Screen Recording was not granted to
`target/debug/easynet-daemon`. Therefore the implementation and executable
runner exist, while E2E-14 remains unpassed until permissions are granted and
both window and application artifacts pass.

Interactive-input implementation update (2026-08-27): browser wheel events
are now mapped through the exact object-contain media rectangle, normalized
from DOM line/page modes to bounded CSS pixels, and translated once at the
device boundary into macOS pixel, Windows native wheel-unit, or Linux X11
detent semantics. Pointer motion is latest-value coalesced to one update per
browser presentation interval before entering the reliable ordered control
channel, preventing obsolete coordinate queues while preserving reliable
button/key release ordering. Unit and component coverage proves wheel
normalization, black-bar rejection, current target geometry/focus epochs, and
latest-coordinate pointer coalescing. This strengthens the implementation but
does not replace the required live, independently observed OS-effect and
latency artifacts.

The same update centralizes the supported `KeyboardEvent.code` vocabulary as
one plugin-owned physical-key contract. macOS and Windows now preserve sided
Command/Windows, Control, Alt/Option, and Shift keys, while macOS, Windows, and
Linux adapters cover navigation, F1-F12, and numeric-keypad keys. Platform
adapters remain responsible only for native keycode/virtual-key/keysym
translation. These mappings are implementation evidence; Windows/Linux still
require their real-host OS-effect campaigns.

Transport-recovery implementation update (2026-08-27): failure of a
previously connected Browser PeerConnection no longer ends the RemoteApp
session. The frontend retires only the failed transport, retains the selected
Resource subject, consent, lease, event watch, and input authority, then asks
the daemon for a fresh transport epoch on the same session. A browser-owned
three-attempt increasing-backoff state machine bounds automatic replacement;
terminal, offline, and reset transitions cancel it, explicit Retry resets it,
and only acknowledged presented media replenishes the automatic budget. A
transient `disconnected` state receives a five-second recovery grace: returning
to `connected` cancels replacement, while grace expiry enters the same bounded
replacement state machine.
Exhaustion preserves the session for explicit Retry rather than manufacturing
a terminal outcome. Component coverage proves same-session/new-epoch
replacement without a second consent or `create_session`; live long-outage and
cross-device recovery evidence remains required.

Linux display is diagnostic-only until a live decoded-frame host artifact
certifies the xcap/OpenH264 baseline; capability metadata is not certification.

| Requirement | Current status | Evidence that exists | Evidence still required before product-complete |
|---|---|---|---|
| Application/window selection and stable capture across macOS/Windows/Linux | Partial | macOS inventory now publishes one application Resource across displays with exact committed window IDs, display IDs, union geometry, ordered front-to-back surfaces, and a deterministic surface-layout epoch; ScreenCaptureKit resolves only owner-matched committed windows into desktop-independent per-window streams and a bounded BGRA `MultiAppSurface` compositor with deterministic black gaps, z-order, monotonic composite timestamps, and one canonical VideoToolbox/WebRTC track; complete-plan rebind supports window-set, geometry, z-order, and cross-display churn while keeping identity and surface-layout proofs separate; the replacement plan is prestarted behind a muted output generation and only selected around a successful Runtime binding commit, while stale commits restore the old generation; capability projection reports `multi_surface` and `multi_display=true`; host decoded-frame harnesses exist. Windows/Linux xcap inventory uses process-stable application identity and a committed exact window-id set; exact window capture rejects owner drift; exact application capture composites the committed set into a bounded virtual-desktop union with black gaps; WebRTC may use the exact xcap/OpenH264 baseline without display widening; platform rows remain `baseline_ready` where live certification is absent | Live macOS/Windows/Linux `remoteapp-cross-platform-capture-e2e.sh` artifacts proving actual OS discovery/capture/rebind behavior, decoded cross-display `MultiAppSurface` frames, layout-only rebind, no pre-commit or stale-generation frame leakage, no display fallback, selected sentinel presence, unrelated sentinel and display-pixel absence, selected Resource URA subjects, and visible terminal receipts |
| Mouse/keyboard input injection is controllable, low-latency, and permission-correct | Partial | App/window sessions without explicit input-control consent downgrade to `view_only`; consented macOS window/application sessions use guarded `target_local`; every pointer/key event revalidates identity, visibility, focus, geometry, and committed application surface state immediately before OS injection; pointer execution also proves that the mapped host point hits the current topmost committed target window, so application-union black gaps and foreign-window occlusion reject as typed failures without posting a `CGEvent`; the frontend coalesces obsolete pointer motion to the latest coordinate once per presentation interval, maps pointer/wheel input through the exact object-contain target rectangle, and normalizes wheel deltas into bounded CSS pixels before one platform translation; Windows User32 `SendInput` and Linux X11/XTest guarded baselines exist, while Wayland remains fail-closed; session views, frontend sending, client sequence/latency telemetry, runtime permission blocking/restoration, diagnostic InvokeBidi parity, and live E2E artifact validators are implemented | Live `remoteapp-input-injection-e2e.sh` artifact proving accepted target effects plus zero OS effect for black-gap and foreign-window-occlusion probes, focus/geometry/identity binding, permission and consent correctness, monotonic sequence application, bounded latency, independent OS observation, selected Resource/session binding, and visible terminal receipt |
| Audio/video codec, frame rate, bitrate adaptation, and drop policy are product-ready | Partial | macOS ScreenCaptureKit captures screen and system audio in one stream; VideoToolbox H.264 and 48 kHz stereo 20 ms Opus frames are sent as separate tracks on one WebRTC PeerConnection; direct WebRTC now parses the browser RFC 6184 Baseline receive format before peer construction and applies `profile-level-id`/`max-recv-level` plus `max-fs`/`max-mbps`/`max-br` as one resolution/FPS/bitrate/OpenH264-level constraint, with an actual encoded SPS test proving negotiated Level 3.1 output; audio capture and sender queues are bounded and expose drop/backpressure counters; daemon stats distinguish negotiated `audio_ready` from positive `audio_media_observed`; frontend explicitly negotiates receive-only audio only when the exact created session reports capture/send readiness, combines audio/video tracks, exposes autoplay recovery, and reports decoded audio packet/sample observations; ABI v8 raw server streams preserve Runtime-owned lifecycle with strict symbol/feature negotiation and v7 fallback, while RemoteApp interactive media correctly remains on WebRTC/binary InvokeBidi rather than a parallel ABI media tunnel; `host-remoteapp-media-adaptation-e2e.sh` executes the real Browser lifecycle across baseline/degraded-network/backpressure with mandatory fixture reset and canonical evidence aggregation | Live `remoteapp-media-adaptation-e2e.sh` artifact proving negotiated H.264/Opus, payload content types, transport, requested/effective/measured FPS, target and observed bitrate, keyframe cadence, bounded frame latency, positive decoded host audio, baseline/degraded-network/backpressure scenarios over the same selected Resource URA and media pipeline, bounded queue depth, explicit stale-frame drop policy, adaptation/drop events bound to the selected Resource URA, session id, and media pipeline id after impairment, decoded video/audio render probe bound to the same pipeline with payload fingerprints after adaptation events, selected Resource URA session subjects, and visible terminal receipts |
| Multi-window/multi-application independent tracking works as an execution effect | Partial | Target tracker state machine, move/resize/loss/rebind events, cross-display application window-set rebind, exact committed-window ScreenCaptureKit multi-stream composition, and complete capture-plan replacement; frontend session details surface target tracking recovery state and a Refresh targets CTA; implementation gates cover black gaps, z-order, exact window admission, and cross-display observer rebind; `remoteapp-multi-window-tracking-e2e.sh` defines the live independent tracking artifact contract | Live `remoteapp-multi-window-tracking-e2e.sh` artifact proving independent concurrent window streams, distinct Resource/session/stream/media identities, decoded-frame isolation and sentinel binding, move/resize geometry revisions, cross-display application churn with pending media rebind and `TARGET_REBOUND`, committed-set rendering, uncommitted-window absence, target loss/rebind behavior, selected Resource URA subjects, and visible terminal receipts |
| Disconnect/reconnect, session resume, consent revoke, cancel, timeout are complete | Partial | Lease monitor, refresh/end session, explicit `end_session` and lease-timeout `terminal_receipt` projection, host `host-remoteapp-session-timeout-e2e.sh` entrypoint creates a short-lived session through the CLI, observes `session_expired` through public `show_session`, and verifies post-timeout `end_session` idempotency preserves the original terminal receipt; host `host-remoteapp-session-cancel-e2e.sh` entrypoint creates a live-target session through the CLI, invokes public `remote_desktop.end_session` with `user_cancelled`, observes the closed state through public `show_session`, and verifies repeated `end_session` preserves the original cancel terminal receipt; host `host-remoteapp-permission-revoke-e2e.sh` entrypoint creates a live-target session and waits for real platform permission revoke evidence before accepting public `show_session` projection of `target_permission_revoked`, revoked consent, ordered `TARGET_PERMISSION_REVOKED`/`MEDIA_SOURCE_LOST`/`SESSION_CLOSED` events, and terminal receipt binding; host `host-remoteapp-session-resume-e2e.sh` proves only lease refresh survival by waiting past the original lease and validating the same non-terminal session through public `show_session`; `frontend-remoteapp-browser-lifecycle-e2e.sh` has a real transport-resume path that verifies same-session survival, old PeerConnection retirement, a newer daemon-issued transport epoch, a new connected PeerConnection, `watch_events` reattachment, decoded media after resume, and unchanged input authority; its 2026-08-26 live macOS window run passed across `ONLINE/J700 -> UNKNOWN/C440 -> ONLINE/J700`, preserved session `rdp-6a0f55cef39688993fba6f06`, replaced epoch `1787686710123091` with `1787686896984117`, rendered one `1688x1080` post-resume frame, preserved the explicit Accessibility policy block, and ended with a visible `caller_ended` receipt; the target-monitor worker-only campaign preserved feature daemon PID `62280`, public session `rdp-66843994a4396d038cc76b94`, selected Resource, consent/binding/transport/media epochs, ordered public and durable generation `1 -> 2` worker events, a later Browser frame, and a visible `caller_ended` terminal receipt; frontend retention of closed `end_session` views with terminal receipt and cleared session token, permission revocation terminates the daemon session with a `terminal_receipt` and frontend terminal sync, target loss and transport failure taxonomy, frontend watch_events recovery handling for degraded and permission-revoked sessions, frontend exposes `Retry session` for daemon retry-session guidance, terminal receipt retention no longer blocks new `create_session`, frontend preserves non-terminal RemoteApp sessions across device-offline presence drops and rebinds through `show_session` + WebRTC restart, canonical SDK cancel/timeout semantics, daemon-local recovery snapshots persist session token/target/consent/event facts, plugin startup rehydrates snapshots into degraded session rows, recovery batch loading isolates a bad row without dropping valid rows, Unix recovery files are private for persisted session tokens, rehydrated non-terminal sessions can start a fresh media negotiation epoch without minting a new session id, rehydrated non-terminal sessions re-enter target monitoring, target monitor desired tracking state survives worker replacement, recovery startup synchronously settles snapshots whose lease elapsed while the daemon was down as durable `session_expired` terminal rows, recovery snapshots preserve session-local runtime input permission blockers for public `show_session` after plugin restart, runtime coverage proves public `show_session`, `watch_events`, and `end_session` operate on a rehydrated row, and the full `remoteapp-crash-restart-recovery-e2e.sh` matrix remains the aggregate gate even though active daemon SIGKILL, Unix stale-socket cleanup, close-crash replay, and one macOS/window worker-only recovery child now have real proofs | Long-outage and network transport reconnect handoff E2E, live consent revoke termination E2E pass report from a real host permission revoke, canonical Axon cancel/timeout receipt-chain evidence for RemoteApp abilities, live aggregate `remoteapp-crash-restart-recovery-e2e.sh` artifact proving all supported daemon/plugin recovery scenarios, recovered WAL/idempotency/replay-guard/lock state, Windows named-pipe restart lifecycle if supported, cross-device recovery, and visible terminal receipts |
| NAT/relay/WebRTC/direct fallback network paths are verified | Partial | Typed host/STUN/TURN/EasyNet relay configuration, daemon `client_ice_servers`, frontend consumption, native selected-pair projection, and a fail-closed verifier. The Browser runner captures the real selected pair before its rendered-frame observation and binds it to User/Agent Invocation identity, provider Device host, Browser peer, Resource and session; the verifier requires production set_description/report_client_state rather than diagnostic attach. Reproducible host runners now prove constrained direct, TURN relay, and Hub-owned EasyNet relay children. The EasyNet child uses a session/resource-bound Hub lease rather than daemon environment credentials, has three server-observed coturn allocations, selected/nominated/succeeded relay ICE, three post-selection rendered frames, redacted lease evidence, a visible terminal receipt followed by HTTP 409 same-binding reacquire rejection from the release tombstone, and ordinary Hub/daemon restoration. The STUN-only runner, reflexive candidate-admission instrumentation, server-binding projector, hard deadline, and mutation tests exist and fail closed, but its real srflx child has not connected | One aggregate live `remoteapp-network-fallback-e2e.sh` campaign completing the constrained STUN srflx scenario alongside the passed direct, TURN, and Hub-owned EasyNet relay children, with session-bound selected/nominated/succeeded candidate-pair evidence after applied allowed/blocked route constraints, rendered media bound to the same selected candidate pair after selected-pair observation, redacted credentials, selected Resource URA session subjects, and visible terminal receipts |
| Frontend UI can discover, authorize, start, display, control, and end session | Partial | Frontend subject boundary, dedicated surface gates, component coverage for picker → permission_status preflight → optional request_permission → consent → create → WebRTC audio/video attach → watch_events → end, explicit independent capture/input permission states, capture-denied/restart-required Share/double-click/store-admission blocking, input-denied view-only downgrade, target-scoped WebRTC lifecycle unit coverage, watch_events recovery-state coverage, daemon `input_readiness`, executable target refresh recovery, executable retry-session recovery, H.264/Opus `media_pipeline_support` and browser audio/video observations, autoplay recovery, terminal receipt coverage, permission-revoked terminal sync coverage, target focus-epoch input gating, product-flow harness entrypoint, and a Browser/Tauri lifecycle evidence verifier that requires both permission abilities to remain host-local | Live Browser/Tauri E2E artifact with real backend/runtime proving picker → permission → consent → create_session → WebRTC audio/video attach → watch_events recovery → decoded media presentation → input/control or policy-block → end_session → visible terminal receipt, with every lifecycle step observed by browser/Tauri automation in order; if input is applied, submitted data-channel frame and daemon applied event target_focus_epoch must match the runtime session target focus epoch |
| Cross-device E2E smoke/regression exists beyond local provider boundary | Partial | `remoteapp-cross-device-product-smoke.sh` composes Docker two-node routing and synthetic media/bidi carrier gates, reports source/runtime provenance, aggregates observed caller/provider device topology, and fails completed local-provider-only runs instead of marking them passed; `remoteapp-cross-device-remoteapp-e2e.sh` now accepts only the production create_session/set_description/watch_events/report_client_state/end_session chain, rejects diagnostic attach, binds admitted `caller_ura` + SystemAgent `callee_ura` + execution-host `provider_device_ura` + opaque Browser `client_endpoint_id`, and requires a remote execution boundary, connected PeerConnection/ICE state, a selected candidate pair, H.264 production rendering on that Browser endpoint after connection, and a verified canonical terminal receipt for display/window/application; host-local decoded-frame scripts cover local capture/render decode | RemoteApp-specific two-host or equivalent network namespace E2E with an independently observed Browser client endpoint and provider execution Device, `local_provider_boundary_only=false`, remote target inventory, production WebRTC/media from actual display/window/application capture rather than diagnostic attach, Browser-endpoint rendering after a selected connected ICE pair, input policy, and verified terminal teardown evidence |

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

Latest target-selection and binary-stream verification on 2026-08-26: live macOS
window refresh passed at
`target/e2e/host-remoteapp-target-picker-freshness/20260826-073415-window-live/report.json`
and selected the AppKit sentinel by the authoritative
`owner pid + native window_id` tuple even when Screen Recording privacy hid the
window title and truncated the process display name. A separate live
application refresh passed at
`target/e2e/host-remoteapp-target-picker-freshness/20260826-073402-application-live/report.json`.
It selected a real LaunchServices application Resource by stable bundle/app
identity and owner pid, bound both positive native `resolved_window_ids`,
preserved matching `front_to_back_surfaces`, required a positive
`window_set_epoch`, and proved `display_scoped=false`. The two target kinds now
have distinct product-flow steps and subreports; neither can satisfy the other.
The fixture keeps LaunchServices executables and command/ack IPC in a
fixture-owned physical temporary directory because a launched application does
not inherit terminal privacy access to repository paths under `~/Documents`.
Durable reports remain in the requested output directory, and cleanup validates
the exact runtime directory and process command before terminating or removing
fixture-owned state. All host lifecycle harnesses share one fail-closed
selector; diagnostic labels participate only when no authoritative Resource
URA/PID selector exists. These two live artifacts prove picker freshness and
application multi-window selection identity, not capture, media, input, or
session lifecycle. A current permission/subject recheck at
`target/e2e/remoteapp-permission-subject/20260826-current-audit/report.json`
passed the authority contract and six negative subject cases, while recording
`screen_capture_permission_granted=false` for the exact current
`target/debug/easynet-daemon` executable. The Runtime and Hub are connected;
new legal capture/session lifecycle evidence is blocked by the real macOS
privacy grant rather than by offline routing. Separately, the EasyRemote opt-in
live v8 smoke passed byte-for-byte against the then-current daemon and C ABI
library. The present fixed-frame ABI supersedes that draft projection and is
covered by local Rust/Python/Go conformance; the live smoke must be rerun before
the new carrier is claimed live. The historical payload proof included
arbitrary 16 KiB bytes, embedded NUL bytes, and an empty typed payload. It does
not prove the current fixed-frame carrier or RemoteApp's interactive WebRTC
media plane.

Latest Browser and cross-device execution on 2026-08-26: the real Frontend
Playwright runner loaded the application, authenticated against the live Hub,
opened the Device Access target picker, invoked host-local
`remote_desktop.permission_status`, selected a real window Resource, granted
consent, created a production session, completed `set_description`, connected
WebRTC, attached audio and video tracks, streamed `watch_events`, rendered four
frames, surfaced media-pipeline state, and closed through `end_session` with a
visible `caller_ended` terminal receipt. The Browser User remained the caller,
the device-sponsored Remote Desktop SystemAgent remained the callee, the Device
remained the execution host, and the selected window Resource remained the
subject across the production ability chain. Input correctly failed closed as
`target_focus_permission_missing` because macOS Accessibility was unavailable.
This is a complete local window/UI lifecycle child proof, not successful input
injection, an application/display variant, cross-platform proof, or a remote
device campaign.

A second live Browser run exercised the same window session across an orderly
daemon stop/start. Device presence moved `ONLINE/J700 -> UNKNOWN/C440 ->
ONLINE/J700`; the UI kept the non-terminal session surface visible; stale
callbacks from the retired input channel could not overwrite reconnect state;
the replacement Runtime epoch, PeerConnection, event watch, and rendered frame
were observed by Browser automation; and the formal lifecycle verifier passed
against `/tmp/remoteapp-browser-transport-resume-live-20260826-v7/evidence.json`.
This closes the previously missing local Browser restart-resume child proof,
but not the dedicated four-scenario crash/restart matrix.

A third live Browser run now passes the complete local macOS application
lifecycle. The first strengthened attempt exposed a cross-language authority
bug: FNV-derived `target_identity_epoch` values could occupy all 64 bits, so
JavaScript rounded the value and the frontend correctly refused to invoke
`remote_desktop.focus_target`. Resource-derived window-set/layout epochs are now
canonically constrained to positive JSON-safe integers (`<= 2^53 - 1`) at the
single Runtime derivation source. After rebuilding the daemon, every live
Application Resource satisfied that bound and Browser focus reached Runtime.
The passing artifact
`/tmp/remoteapp-browser-application-live-20260826-v5/evidence.json` binds an
authoritative non-display-scoped Application inventory snapshot to a Runtime
`AppSurface` execution snapshot with `scope_widened=false` and
`display_fallback_used=false`, production WebRTC, one rendered `920x1080`
frame, typed Accessibility permission denial, and a visible `caller_ended`
receipt. The execution window set changed from `[18784]` to `[18784,24147]`
without changing the Application Resource, which is real rebind evidence but
not the independent sentinel/pixel-isolation proof required for full
multi-window certification.

The reproducible TURN child runner then passed one constrained relay-only
Browser session. Coturn observed three allocations; relay-only local SDP and
ICE policy were applied before pair selection; the selected pair was connected,
nominated, and succeeded; bidirectional byte counters were positive; three
frames rendered after pair selection; and terminal cleanup completed. Chrome
materialized the selected local candidate as peer-reflexive, so the proof binds
relay-only policy/local SDP to the independent server allocation rather than
mislabeling the RTCStats candidate type. The constrained direct child also
passed with zero projected ICE URLs and host-only local/remote SDP. The
Hub-owned EasyNet relay child now also passes with a short-lived lease,
three independently observed allocations, a selected/nominated/succeeded relay
pair, three later frames, and terminal cleanup. STUN srflx remains the only
missing route from the four-route matrix. The new STUN runner correctly
rejected a same-host direct selection, then identified Docker Desktop's
hidden-VM return-path limitation. Its revised provider-host STUN + externally
reachable VM-NAT design has an independently observed Binding/mapping proof and
now filters both trickle and embedded offer SDP, but it still has no Browser
selected-pair/media/terminal artifact. None of the topology probes is counted
as a route pass.

The Docker combined cross-device smoke then passed with two distinct caller /
provider Device URAs and `local_provider_boundary_only=false`. Its routing step
passed 58/58 assertions; its synthetic stream/bidi step passed 45/45 assertions,
including remote consumer cancellation, provider release, reacquisition,
unique verified receipt chains, exactly one terminal receipt per completed
operation, and route removal after plugin uninstall. The media fixture was
migrated to descriptor schema v3 (`task` exposure, `media` dedicated surface,
dedicated-surface subject contract, and `json_frames` bidi wire kind). Receipt
tuple validation now resolves the expected callee from catalog `owner_ura`:
the plugin-management Agent is the Ability callee and the Device is only its
execution host. A Service callee remains correct for stable user-owned product
surfaces such as Pages, not for this device system plugin.

This combined artifact is a live lower-bound regression, not release closure:
it is under `/tmp/remoteapp-cross-device-live-20260826-v3`, uses dirty source
revision `f3a02fc0f91c7495284c838925734dfcd2227f53`, and records prebuilt runtime
image `sha256:63dfb06db969e6ee0c3029e9d95baa3772ab358f9327c88632a400863f270703`
created on 2026-08-25. It does not contain real remote OS capture, input effects,
host audio, browser rendering, or direct/STUN/EasyNet relay evidence. The local
TURN child described above is separate and does not change that cross-device
artifact's scope.

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

The 2026-08-28 isolation review further narrows that statement. Linux X11
display-global interaction remains implementable with XCB/XTest, but
Window/Application interaction is now intentionally `view_only`: XTest cannot
bind a complete press-to-release lifecycle to one target, so a focus change can
otherwise release into another application. Linux Window/Application recovery
also fails closed until an X11 `DestroyNotify`-backed window-generation lease
can distinguish same-process XID reuse. Native target inventory is now isolated
in the required sibling `easynet-remoteapp-native-host` process. Independent
inventory and input-guard lanes use bounded mailboxes and strict length-prefixed
IPC; a timeout, stale identity, malformed/oversized response, disconnect, or
shutdown kills and reaps that generation before replacement. Unsolicited extra
responses fail the generation through non-blocking bounded delivery instead of
deadlocking the reader during teardown. Local macOS tests prove real-process
round-trip, parent-liveness exit, oversize rejection, repeated injected hang
recovery, and this extra-response failure path. Unix media capture, encoding
and macOS host audio now run in the separate canonical media-host process with
generation-fenced control/video/audio lanes. Windows video now uses the same
named-mapping/notification architecture in production dispatch and
cross-compiles, but real Windows execution and a signed cross-platform release
campaign remain product blockers.

The target-observation helper boundary no longer links back through the root `easynet` crate. A
plugin-private `easynet-remoteapp-native-protocol` crate owns the versioned DTOs,
validation, and bounded framing; the native-host library owns OS inventory and
parent-liveness execution; the daemon keeps only scheduling, process lifetime,
wire validation, and conversion into session-domain observations. The old
daemon-side helper server and platform inventory implementation were removed.
The dependency-graph gate rejects any native-host path back to Runtime, Axon, or
tonic. Host-audio capability probing now has an independent one-shot
`easynet-remoteapp-media-host` capability failure domain and
`remoteapp_media_host_v1` capability schema. The daemon commits a capability snapshot
only after a valid response and successful child exit; timeouts are killed and
reaped. Windows/Linux capability mode returns hosted-session audio unavailable
without converting available OS audio primitives into a product claim.
This establishes process/package independence for target observation,
capability discovery, and hosted screen capture/encoding. It does not establish
Windows live parity or cross-device product readiness.

Latest 2026-08-27 revalidation used a freshly rebuilt Linux runtime bundle,
fresh Hub image, current macOS CLI/daemon, and real Browser automation. The Hub
started healthy, the host re-paired, and Device presence reached
`directory_status=online` plus `session_admitted=true`; this directly resolves
the previously observed stale-runtime Offline condition. The EasyNet relay
refresh/resume runner also exposed and fixed a `pipefail` readiness race where
`docker logs | grep -q` misclassified a ready coturn process after SIGPIPE.
After that fix an initial Browser run reached authentication, target inventory,
picker, host-local permission status, consent routing, and create-session
dispatch before Runtime rejected the missing Screen Recording grant. That run
exposed a frontend seam: an already-known capture denial still allowed consent
and create-session admission. The frontend now projects capture and input as
independent explicit states, blocks Share/double-click/store admission for
capture `blocked` or `restart_required`, and downgrades known input denial to
view-only instead of rejecting media. A repeated real Hub + daemon + Browser
run at
`/tmp/remoteapp-permission-gate-live-20260827/browser/evidence.json` executed
host-local `permission_status` and `request_permission`, observed capture
`blocked` and input `granted`, and terminated before either `grant_consent` or
`create_session`; it claims no relay coverage. The Browser evidence verifier
also requires both permission abilities to remain host-local and rejects a
passing request result unless capture is `granted`. Short-TTL lease rotation
plus same-session resume remains a required live check after the exact daemon
binary receives the OS grant.

`runtime status --json` now exposes an explicit pairing projection with paired,
unpaired, and invalid states plus a separately typed current User state and
canonical User URA. The host permission E2E resolves its default caller only
from that public projection and records
`caller_user_resolution_source=runtime_status_pairing`; it no longer parses
credentials or trust-file layout. A deterministic regression gives the public
status a current User while local identity files contain stale values and
proves that only the status contract selects the caller. The product-completion
gate usage contract also enumerates the separately required Browser transport
resume report, and its regression derives all direct and window/application
lifecycle report variables from the actual required-report declaration to
prevent documentation/gate drift.

The live identity recheck at
`/tmp/remoteapp-permission-current-20260827-v4/evidence.json` observed
`runtime_status_pairing`, the bound current User, and the expected host-local
permission subjects; its outer report failed only because Screen Recording for
the exact current daemon remained denied. A formal production aggregation at
`/tmp/remoteapp-product-completion-current-20260827-v2/report.json` accepted the
current cross-device lower-bound report with two observed distinct Device pairs
and no validation errors, while failing closed on the other 18 missing live
reports and the absent signed campaign. It emitted
`product_complete_eligible=false` and `product_complete_claim=false`.

The 2026-08-27 cross-device lower-bound revalidation at
`/tmp/remoteapp-cross-device-current-20260827-v3/report.json` passed against
EasyRemote revision `1f056ef3bfa537d44348576f7aaf42744ee25523`: the routing
child passed 58/58 assertions, the synthetic media/bidi child passed 45/45,
both observed distinct caller/provider Device URAs, and the aggregate records
`local_provider_boundary_only=false`. This resolves the stale local EasyFlow
SDK fixture problem and reconfirms governed Hub transport across independent
device identities. It remains a lower-bound carrier test: its explicit
coverage is false for real OS application/window capture, pointer/keyboard OS
effects, host audio, Browser rendering, and the direct/STUN/TURN/EasyNet relay
matrix, so it does not change the product-complete status.

The separate cross-device RemoteApp product verifier no longer accepts the
diagnostic `remote_desktop.attach` path as a substitute for product media. Each
display/window/application scenario must now prove
`create_session -> set_description -> watch_events/report_client_state ->
end_session`, connected PeerConnection and ICE states, a selected candidate
pair, H.264 production media rendered on the caller device after connection,
and a canonical verified terminal receipt. The product-completion aggregator
independently rechecks these summaries. The current synthetic lower-bound
artifact cannot satisfy this stronger contract, and no live two-device
RemoteApp artifact exists yet.

## 2026-08-29 — Current Linux route campaign and completion boundary

The rebuilt Linux provider and real Browser runner now pass the focused direct
application child, TURN window and application children, and EasyNet-relay
window and application children. The accepted reports are under
`target/e2e/remoteapp-direct/live-linux-application-docker-browser-20260829-r3`,
`target/e2e/remoteapp-turn-relay/live-linux-{window,application}-20260829-*`,
and `target/e2e/remoteapp-easynet-relay/live-linux-{window,application}-20260829-*`.
Every accepted leaf has `evidence_origin=live_runner`, connected ICE, a route
class constrained by its fixture, rendered Browser frames, production Ability
bindings, and terminal cleanup. The application leaf binds an exact
process-scoped two-window set and never widens to display capture.

The accelerated EasyNet lease run at
`target/e2e/remoteapp-easynet-relay/live-linux-display-refresh-resume-20260829-r5/report.json`
also passes lease rotation plus same-session daemon recovery, replacement
transport, resumed media, and terminal cleanup. Two isolated STUN topologies
produced real RFC 5389 bindings, but both timed out before a connected selected
srflx/prflx pair because their VM/private return path was not viable. Their
failed reports are retained under `target/e2e/remoteapp-stun-srflx`; no STUN
route is claimed.

The platform evidence matrix remains intentionally incomplete. Linux window
and application capture is real, but target-local input is correctly projected
as `view_only` with `target_scoped_keyboard_pointer_dispatch_unsafe`; desktop-
global XTest cannot prove target-isolated press-to-release behavior. The macOS
signed-helper run is blocked by Screen & System Audio Recording permission even
though Accessibility is granted. No real Windows interactive host or VM was
available, so Windows capture and SendInput/UIPI behavior remain unverified.

The media hot-path contract is closed at source and mutation-gate level:
media-host payloads use the fixed shared-memory lane, daemon ingress validates
the mapped lease and performs one bounded detach into transport-owned bytes,
and WebRTC owns the payload for RTP/NACK lifetime. Generic ABI v8 remains a
separate binary Invocation-stream extension and is not accepted as RemoteApp
media-plane evidence.

The final aggregate at
`target/e2e/remoteapp-product-completion/final-20260829/report.json` failed with
20 explicit errors: the signed campaign authority was absent and 19 required
production reports were not supplied. It emitted
`product_complete_eligible=false`, `finalization_state=not_eligible`, and
`product_complete_claim=false`. Its SHA-256 is
`dfbf09a3de6ef061dc0572a5505e747e55fcfe63aeb872dc92710cc0ced6c2d6`.

## Next implementation batches

1. Frontend expansion: repeat the passed local window lifecycle for application
   and display targets, successful input, recovery, and a real remote device.
2. Product input batch: focus-safe pointer/keyboard injection with target epoch
   validation and permission proof.
3. Network batch: retain the passed direct, TURN, Hub-owned EasyNet relay, and
   live short-TTL refresh/resume children; finish constrained STUN before
   claiming the four-route matrix.
4. Cross-platform capture batch: grant and rerun the exact macOS signed helper,
   obtain a real Windows host, and keep Linux Window/Application input
   explicitly view-only until a target-isolated backend exists.
5. Recovery batch: reconnect/session resume, revoke, cancel, timeout,
   crash/restart recovery.
