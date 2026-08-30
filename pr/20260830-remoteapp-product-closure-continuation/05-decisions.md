# Decisions

## 2026-08-30 — Keep product completion fail-closed

The existence of broad implementation code and test harnesses is not treated
as product completion. Missing native platform, network topology, or
cross-device evidence remains an explicit incomplete row.

## 2026-08-30 — Keep generic ABI v9 out of the RemoteApp media plane

RemoteApp continues to use the plugin-private leased shared-memory lane and
WebRTC. ABI v9 improves generic native Ability streams but does not replace the
codec, congestion-control, RTP/SRTP, or ICE data plane.

## 2026-08-30 — Preserve concurrent work

No bulk formatting, staging, commit, reset, or cleanup is allowed while the
current cross-repository worktrees contain unattributed concurrent changes.

## 2026-08-30 — Execute v9 as the generic native binary-stream ABI

ABI v9 is the supported zero-base64 native carrier. It retains canonical frame
metadata and Runtime-owned authority, sequencing, receipts, terminal state, and
error semantics. Payload memory is leased with explicit release; SDKs must copy
or wrap it in an owned guard and must never expose an untracked borrowed slice.

The lifecycle contract is fail-closed: close and handle destruction wait for an
in-flight callback to return, suppress later callbacks, and purge outstanding
leases. Reentrant close from the callback suppresses later delivery without
self-waiting. Queue admission and delivered leases consume one bounded byte
budget, and active stream counts are bounded per Runtime handle and process-wide.

## 2026-08-30 — Keep v9 additive and ship an executable implementation

Older v7/v8 discovery documents remain valid. A runtime that advertises v9 must
advertise its complete capability and symbols. Release archives must contain the
native C-ABI library, not only headers/specifications, and the staged library's
entire global export table must equal the versioned v9 allowlist before signing.

## 2026-08-30 — Do not equate v9 completion with RemoteApp completion

ABI v9 removes JSON/base64 from generic native Ability streaming and is useful
for snapshots, encoded chunks, files, and control/data records. It is not the
RemoteApp sustained media transport: interactive desktop video/audio continues
to require the existing WebRTC/shared-memory lane for congestion control,
packetization, encryption, backpressure, and realtime adaptation.

## 2026-08-30 — Keep language-native ownership native

Go and Python cross the C callback boundary, so their v9 APIs expose explicit
leased payload objects and preserve the old v8 owned-event API separately.
Rust already receives protobuf payload bytes in the same ownership domain; its
public facade moves them into `Vec<u8>` and therefore must not simulate retain /
release leases. This is owned-byte transport, not a claim of end-to-end network
zero-copy: tonic/prost may still copy during decoding.

## 2026-08-30 — Exercise v9 through the authority-owning SDK seam

A live v9 call is not permitted to bypass `RuntimeAbilityClient`. The ability
client owns descriptor resolution, User-to-descriptor-bound-Resource subject
projection, and typed authority metadata binding; the v9 carrier owns only the
raw payload lease and stream lifecycle. This keeps transport representation
independent from Invocation authority and prevents product clients from
recreating the subject/delegation rules.

## 2026-08-30 — Introduce one private native-platform authority port

Process generation and native window ownership are product/platform facts, not
SDK runtime concepts and not media-host implementation details. A private
RemoteApp platform crate owns Windows FILETIME identity, Linux XRes plus
`/proc` identity, and the pure capture-eligibility predicate. Discovery,
observer, media, focus, and input consume it; duplicated implementations and
advisory PID fallbacks are removed.

## 2026-08-30 — Model browser session work as generations and aggregates

The Frontend must distinguish creation, active, closing, and terminal ownership.
Closing the UI invalidates the creation generation; a late successful create is
ended using its returned immutable subject/token/causal context. Ambiguous end
keeps the aggregate and reconciles through `show_session` plus replayed events.

## 2026-08-30 — Separate leaf convergence from product integration proof

The native-platform leaf and its mutation gate are necessary but not sufficient
for SPEC conformance. The authoritative platform branch must also pass the
existing application membership/stacking boundary and the complete main-crate
implementation filter matrix. A new process-instance invariant requires all
Windows fixtures and observers to carry the same canonical identity; weakening
the resolver to preserve an old fixture is forbidden.

Committed application capture must prove both exact committed-surface presence
and the absence/order of unexpected process-owned eligible windows. Ignoring
new uncommitted surfaces is safe for leakage but is not enough to prove the
session's application window-set epoch remains current; target tracking and the
media-process pre-capture guard must converge on one explicit policy.

## 2026-08-30 — Keep aggregate observations cross-platform optional

`AppWindowObservation.pid` remains optional because macOS enumeration can
legitimately lack an owner PID. The authoritative xcap branch filters out rows
without a canonical process instance, so it projects the narrowed PID as
`Some(pid)` instead of weakening the shared aggregate type or restoring an
advisory PID fallback.

## 2026-08-30 — Format platform errors at the backend-failure boundary

Linux target invalidation accepts diagnostic values through `Display`, matching
the macOS backend boundary. This lets `anyhow::Error` retain its diagnostic text
when it is classified as `TargetInvalidated`; the internal `BackendFailure`
continues to own sanitization and bounded string storage.

## 2026-08-30 — Type exact unary route terminals at registration

Daemon exact unary providers return JSON bytes by contract. Their atomic boot
registration must therefore use Axon's typed batch surface and declare
`application/json` before terminalization. The Go SDK remains strict and does
not infer JSON from an `application/octet-stream` payload.

## 2026-08-30 — Treat warning-free builds as configuration ownership

RemoteApp fallback modules and fixtures compile only in the feature/platform
matrix that owns their production call sites and tests. Native-media tests do
not force unrelated fallback implementations into the crate. Genuinely unused
helpers are removed instead of retained behind `allow(dead_code)`; the fallback
feature matrix is compiled explicitly before and after the change so this does
not erase supported behavior.

## 2026-08-30 — Let WebRTC fragment native macOS H.264 NAL units

VideoToolbox's maximum H.264 slice-byte property is not supported by every
physical encoder. The macOS media contract therefore permits a NAL up to the
bounded access-unit size and delegates RTP FU-A fragmentation to the WebRTC
packetizer. This preserves the bounded payload contract without making an
optional hardware tuning property a session-terminating prerequisite.
