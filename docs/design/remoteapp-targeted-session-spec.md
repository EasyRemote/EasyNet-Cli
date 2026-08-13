# EasyNet RemoteApp Targeted Session SPEC

Status: draft target architecture
Scope: EasyNet-Cli daemon, builtin remote desktop plugin, frontend execution surface
Primary goal: make `application`, `window`, and `display` remote sessions functionally distinct and verifiable.

Design posture: this is a functional SPEC, not a naming refactor. A patch that
only renames surfaces, adds descriptor metadata, or forwards the old display
capture through a new UI does not satisfy this document.

## 1. Problem statement

EasyNet must not treat remote desktop as only a display-level feature with application/window metadata attached. The intended product behavior is:

```text
User selects one target:
  display
  window
  application

EasyNet creates one scoped remote session:
  scoped consent
  scoped target binding
  scoped capture source
  scoped input policy
  scoped lifecycle events
  scoped audit evidence
```

If a user selects a window or application, the implementation must not silently capture the whole display, crop it, mask it, or fall back to it. That would expose unrelated content and would make the frontend selection a UI fiction.

## 2. Current implementation evidence

The current tree already contains useful foundations:

- `ResourceType` already includes `Display`, `Application`, and `Window` in `src/daemon/persistence/resources.rs`.
- `remote_desktop.create_session` already resolves the acted-on target from the invocation envelope subject in `plugins/remote-desktop/src/handlers/create_session.rs`.
- `plugins/remote-desktop/src/resource.rs` already allows display/window/application resource subjects for remote desktop sessions.
- macOS resource bootstrap already enumerates window/application targets through `CGWindowListCopyWindowInfo` in `src/daemon/ability/builtins/resources/media/resource_bootstrap.rs`.
- macOS ScreenCaptureKit native capture already branches by `ResourceType::Display`, `ResourceType::Window`, and `ResourceType::Application` in `plugins/remote-desktop/src/screencapturekit_capture.rs`.
- WebRTC native media now starts ScreenCaptureKit from the session-owned `RemoteAppTargetBinding` in `plugins/remote-desktop/src/transport/webrtc_native_media.rs`.
- `remote_desktop.watch_events` already exposes a session event stream in `plugins/remote-desktop/src/handlers/watch_events.rs`.

Remaining implementation gaps for the product effect:

- `remote_desktop.*` is still published under the `media` frontend surface, not a dedicated remote desktop surface.
- `meta.list_resources` reads the persisted resource table; it does not guarantee live refresh at the time the frontend asks for current windows/applications.
- The frontend picker path has not yet been proven to call `resource.refresh_remote_targets` and use the refreshed `resource_ura` as `Invocation.subject`.
- Decoded-frame E2E evidence is still required to prove that window/application sessions never leak full-display content.
- Full rebind and multi-display application sessions are not complete.
- Interactive app/window input must remain view-only until focus validation, coordinate mapping, and target epoch checks are proven on the execution path.
- Transport only has local host candidate discovery today; production remote usability needs STUN/TURN/EasyNet relay state.
- Clipboard and file-drop frame types exist in the input model but are not implemented and must remain explicitly unsupported until split into separate abilities.

## 3. Architecture invariants

These are normative requirements.

```text
target_binding is the authorization boundary,
capture boundary,
input boundary,
media source boundary,
lifecycle boundary,
and audit boundary.
```

Identity and ownership:

```text
User Principal = caller / accountability root
Device = host, sponsor, key custodian
SystemAgent = owner of device-native remote_desktop.* AbilityDescriptors
RemoteDesktopPlugin = AbilityImpl
Target Resource = display/window/application subject
```

Forbidden model changes:

- Do not model the user account as an Agent.
- Do not model the plugin as an Agent or Principal.
- Do not make the Device the public callee for the ability.
- Do not expose `remote_desktop.*` through a generic ability button.
- Do not let app/window sessions fall back to display capture.

## 4. Dedicated frontend surface

Add a dedicated surface:

```rust
AbilityDedicatedSurface::RemoteDesktop
BuiltinPluginFrontendContract::OPERATOR_REMOTE_DESKTOP
```

Every `remote_desktop.*` descriptor must declare:

```toml
dedicated_surface = "remote_desktop"
subject_contract_kind = "dedicated-surface"
```

The current `media` surface is not precise enough. Remote desktop is a lifecycle surface with permissions, target binding, signaling, media, input, and target events. It is not a normal media snapshot/subscribe button.

## 5. Frontend lifecycle

The frontend must execute this lifecycle, in order:

```text
list/watch targets
-> permission_status
-> request_permission if needed
-> grant_consent(subject = selected target resource)
-> create_session(subject = same target resource, causal_context = consent receipt)
-> WebRTC signaling
-> attach/watch_events
-> refresh_lease/end_session
```

Subject rules:

```text
permission_status/request_permission:
  subject = caller user self
  or descriptor-bound invoke subject
  descriptor/admission must explicitly allow that subject shape

grant_consent/create_session:
  subject = selected display/window/application resource URA

set_description/add_ice_candidate/watch_events/attach/refresh_lease/end_session:
  subject = session URA when the ability acts on session lifecycle
  or the original selected target resource URA when the existing descriptor
  contract still requires resource subject; the surface adapter must make the
  choice explicit and must not put subject in args
```

The frontend must never pass `subject` in JSON args.

The dedicated frontend surface must hide or reject `remote_desktop.*` from the
generic media/action surface. A generic ability dialog is not allowed to guess
the subject for remote desktop lifecycle abilities.

## 6. Target enumeration and refresh

The product must expose current remoteable targets:

```text
display
window
application
```

Acceptable API options:

```text
resource.list_remote_targets(types=["application", "window", "display"])
```

or:

```text
resource.refresh_remote_targets
resource.watch_remote_targets
```

Normative behavior:

- `meta.list_resources` remains a read of the resource registry/cache. It may
  show cached targets and freshness metadata, but it must not silently perform
  host-local discovery and mutate `resources.json` as a side effect.
- `resource.refresh_remote_targets` is the host-local inventory refresh ability.
  It may atomically update the resource cache and must return the live projection
  it just observed. Partial refresh failure must leave old rows either marked
  `unavailable` with a typed reason or excluded from the live projection.
- `resource.refresh_remote_targets` belongs to the daemon resource inventory
  layer. Boot-time `resource_bootstrap` may seed resources, but it must not be
  the only freshness mechanism for the remote desktop picker.
- `resource.watch_remote_targets` is the inventory delta stream. It reports
  target inventory changes, not per-session binding lifecycle events.
- `remote_desktop.create_session` consumes one selected resource subject and
  re-resolves only that subject for binding. It must not enumerate all targets
  or mutate `resources.json`.
- The frontend remote desktop target picker must use a live target inventory
  ability, or a two-step flow:

```text
resource.refresh_remote_targets
-> meta.list_resources(types=["application", "window", "display"])
```

- The persisted `resources.json` table is not sufficient as live binding state.
  It is a stable subject registry plus cached metadata.
- A window resource is ephemeral. An application resource may be stable, but each session must resolve it to the current OS target identity epoch.
- If live refresh fails, stale auto-discovered window/application rows must be marked unavailable or withheld; they must not be returned as definitely capturable.
- Live target discovery is resource-inventory responsibility, not remote
  desktop plugin responsibility. The remote desktop plugin consumes resource
  subjects and re-resolves them before binding.
- If `resource.refresh_remote_targets` returns live projection rows, the picker
  should use those rows directly. The two-step refresh/list flow exists only
  when the public inventory ability is implemented as refresh receipt plus cache
  projection.

Minimum target projection:

```json
{
  "resource_ura": "easynet:///r/localhost/resource/device.abc/streams/window.macos.123",
  "type": "window",
  "display_name": "Cursor - EasyNet-Cli",
  "owner_agent": "easynet:///r/localhost/agent/device.abc.runtime-resources",
  "host_device_ura": "easynet:///r/localhost/device/abc",
  "binding": "local_device",
  "metadata": {
    "platform": "macos",
    "capture_target": "window",
    "window_id": 123,
    "pid": 456,
    "app_name": "Cursor",
    "title": "EasyNet-Cli",
    "x": 100,
    "y": 80,
    "width": 1600,
    "height": 1000,
    "lifecycle_epoch": 17,
    "availability": "available",
    "supported_capabilities": ["capture", "pointer"],
    "freshness": {
      "observed_at_ms": 123456789,
      "stale_after_ms": 123457289,
      "source": "live_refresh"
    }
  }
}
```

## 7. Target binding

`remote_desktop.create_session` must resolve the selected resource into a live target binding before the session becomes active.

Required create-session response field:

```json
{
  "target_binding": {
    "subject_ura": "easynet:///r/localhost/resource/device.abc/streams/window.macos.123",
    "target_kind": "window",
    "binding_id": "tb_...",
    "binding_epoch": 1,
    "target_identity_epoch": 42,
    "target_geometry_revision": 7,
    "media_source_epoch": 1,
    "consent_epoch": 3,
    "platform": "macos",
    "backend": "screencapturekit",
    "capture_scope": "WindowSurface",
    "input_scope": "view_only",
    "resolved_identity": {
      "window_id": 123,
      "pid": 456,
      "app_name": "Cursor",
      "title": "EasyNet-Cli"
    },
    "bounds": {
      "x": 100,
      "y": 80,
      "width": 1600,
      "height": 1000
    },
    "production_ready": true
  }
}
```

Required semantics:

- If a window target cannot be resolved to the requested window, fail closed.
- If an application target cannot be resolved to a current application window set, fail closed or return `unsupported` with typed reason.
- If the backend can only capture the display, fail closed for app/window sessions.
- If the resolved target changes identity during session creation, fail closed and require the frontend to retry with fresh resources.
- `create_session` must not insert the session row until target resolution succeeds. A session row without a resolved target is not a remote app session; it is an ambiguous negotiation artifact.
- The session profile must persist the returned `RemoteAppTargetBinding`.
- Native media and input must consume the persisted binding, not rediscover independently from a stale `ResourceEntry`.
- Display resolution must not silently bind the first display when the selected display identity is missing or mismatched. First-display behavior is allowed only for an explicit `primary_display` subject.

Required implementation boundary:

```rust
trait RemoteAppTargetResolver {
    fn resolve_for_session(
        &self,
        entry: &ResourceEntry,
        requested_mode: RemoteDesktopMode,
    ) -> Result<RemoteAppTargetBinding, RemoteAppTargetError>;
}

trait RemoteAppMediaSourceFactory {
    fn start_from_binding(
        &self,
        binding: &RemoteAppTargetBinding,
        request: MediaStartRequest,
    ) -> Result<RemoteAppMediaSource, RemoteAppTargetError>;
}
```

Binding ownership invariant:

```text
The session aggregate owns exactly one active target_binding at a time.
create_session resolves and stores this binding before inserting an active session row.
attach/set_description/WebRTC/native media consume this stored binding by binding_id.
They must not re-resolve ScreenCaptureKit targets from ResourceEntry except through
an explicit Rebinding transition that creates a new binding_epoch.
```

OOP module ownership:

```text
daemon resources inventory:
  owns ResourceEntry persistence, resource.refresh_remote_targets, resource.watch_remote_targets
  does not own remote desktop sessions, media streams, input, or rebind policy

remote_desktop::target:
  owns CaptureTargetIdentity, ResolvedCaptureTarget, RemoteAppTargetBinding,
  RemoteAppTargetResolver, RemoteAppTargetError, ScopeAudit

remote_desktop::session:
  owns RemoteDesktopSessionAggregate, session lifecycle, lease, consent,
  target binding sub-state, signaling state, transport state, event log

remote_desktop::transport:
  owns WebRTC endpoint lifecycle and media-source startup
  consumes RemoteAppTargetBinding through RemoteAppMediaSourceFactory
  does not resolve ResourceEntry into native targets

remote_desktop::input:
  owns input frame parsing and platform dispatch
  consumes target binding + tracker snapshot for coordinate/focus validation

remote_desktop::view:
  owns JSON projection only
  does not perform policy decisions, target resolution, or resource refresh
```

Owner projection rule:

```text
owner_agent MUST be an Agent/SystemAgent URA.
Device identity may appear only as sponsor_device_ura or host_device_ura.
Device URAs must not appear in owner_agent, caller, or callee fields.
```

Required aggregate shape:

```rust
struct RemoteDesktopSessionAggregate {
    profile: RemoteDesktopSessionProfile,
    target: RemoteAppTargetBindingStateMachine,
    consent: RemoteDesktopConsentState,
    lifecycle: RemoteDesktopSessionStateMachine,
    signaling: RemoteDesktopSignalingState,
    transport: RemoteDesktopTransportState,
    events: RemoteDesktopEventLog,
}
```

The aggregate constructor must receive a resolved `RemoteAppTargetBinding`.
Constructing a remote app/window session from only `subject_ura` and
`subject_type` is forbidden because it preserves the current seam.

Constructor lifecycle:

```text
remote_desktop.create_session:
1. validate invocation subject
2. consume and validate consent receipt
3. resolve selected ResourceEntry into RemoteAppTargetBinding
4. construct RemoteDesktopSessionAggregate from RemoteDesktopSessionInitWithBinding
5. insert the session row atomically
6. return session view containing target_binding, latest_target_diagnostic, and scope_audit
```

No `RemoteDesktopSession` row may exist without a resolved binding. If
pre-binding state must be represented, it belongs to a separate
`RemoteDesktopSessionCreationWorkflow`; it must not be exposed as an active
remote desktop session.

Engineering cutline:

- Domain objects own invariants; handlers orchestrate use cases.
- `ResourceEntry` is a persistence/projection DTO, not a media-domain object.
- JSON response fields are audit projections, not the source of truth.
- Error classification belongs to typed domain errors, not `anyhow` string parsing.
- Platform-specific native locators stay behind resolver/media-source traits.
- Tests should inject fake resolvers/media factories instead of booting real SCK for every unit boundary.

Complexity and resource budgets:

Let:

```text
W = visible windows
A = running applications
D = displays
R = persisted resource rows
S = active remote desktop sessions
T = terminal/tombstone session rows
E = retained events per session
C = changed targets in one inventory observation
I = ICE candidates per side/session
Q = bounded media/input queue depth
```

Required bounds:

```text
meta.list_resources:           O(R) time, O(result) projection memory, no mutation
live inventory refresh:        O(W + A + D + R) time, O(W + A + D + R) memory
resource cache mutation:       O(R + W + A + D), not O(R * (W + A + D))
exact target resolution:       O(1) by stable native id, O(k) only within one indexed ambiguity bucket
session insert/lookup:         O(1) expected by session_id
terminal row retention:        T <= 4S
target event retention:        O(E) per session, fixed upper bound
tracker observation fanout:    O(C + affected_sessions), not O(S * (W + A + D))
resource cache atomic update:  O(W + A + D), coalesced per refresh
ICE candidates:                I <= 64 per side/session
SDP payload:                   fixed byte cap, default <= 1 MiB
candidate payload:             fixed byte cap, default <= 4 KiB each
media/input queues:            O(Q), explicit bounded capacity
```

The OS live enumeration itself is the lower bound. The design must not multiply
that cost by active sessions. Use a shared host target observer/cache for
current platform snapshots, while each session-level TargetTracker validates
only its bound identity against those snapshots.

Forbidden complexity patterns:

- Full `SCShareableContent` / `CGWindowListCopyWindowInfo` enumeration once per active session per tick.
- Scanning every window for every session when exact native ids are present.
- Calling linear `upsert_resource` for every discovered target without an indexed mutation pass.
- Holding `RemoteDesktopSessionStore` locks while enumerating OS targets, starting ScreenCaptureKit, creating encoders, or negotiating WebRTC.
- Unbounded event logs, event payload bytes, frame queues, target-observation histories, SDP payloads, or ICE candidate vectors.
- Persisting `resources.json` on every tracker tick. Cache writes must be refresh-driven or coalesced.
- Copying video frames into session state, event logs, diagnostics, or resource cache.
- Emitting one session event per rejected high-rate input frame. Rejections must be coalesced or rate-limited.

Initial performance gates:

```text
target refresh with 200 windows: p95 <= 500 ms on local macOS dev hardware
cached exact create_session target resolution: p95 <= 50 ms
forced live create_session target resolution: p95 <= one inventory refresh + 50 ms
target tracker steady-state memory: O(S * E) plus one shared O(W + A + D) snapshot
event log per session: fixed capacity, default <= 256 events
native media frame queue: bounded by explicit video constraints
ICE candidates per side/session: default <= 64
```

These gates are engineering guardrails, not protocol semantics. If the OS API is
slower, the implementation must surface timing diagnostics rather than hide
unbounded work behind a generic failure.

No entry-based production path:

```text
After target_binding is introduced, production media and input paths must not call:
  target_for_entry(ResourceEntry)
  input_policy_for_entry(ResourceEntry)
  pointer_target_for_entry(ResourceEntry)
  any equivalent ResourceEntry-to-native lookup

Those conversions are allowed only inside RemoteAppTargetResolver before binding
creation, or inside explicit Rebinding that creates a new binding_epoch.

This restriction covers production WebRTC, diagnostic `remote_desktop.attach`,
preview streams, and future input data channels. Diagnostic paths may be lower
quality, but they may not bypass target binding or display-fallback rules.
```

Recommended project structure:

```text
src/daemon/ability/builtins/resources/
  inventory/
    refresh_remote_targets.rs
    watch_remote_targets.rs
    target_snapshot.rs
    projection.rs

plugins/remote-desktop/src/
  target/
    identity.rs
    binding.rs
    resolver.rs
    errors.rs
    tracker.rs
    macos_screencapturekit.rs
  session/
    aggregate.rs
    creation_workflow.rs
    target_state.rs
  transport/
    media_source.rs
    webrtc_*.rs
  input/
    router.rs
    platform_macos.rs
  view/
    session_projection.rs
    target_diagnostic.rs
```

Names may adapt to the existing module layout, but semantic ownership must not:
resource inventory, target binding, session aggregate, media transport, input
routing, and view projection are separate layers.

The first macOS implementation may wrap the existing CoreGraphics resource metadata plus ScreenCaptureKit `SCShareableContent` enumeration. It must, however, return a typed target binding rather than letting the media path discover failure later.

`resource_ura` is the invocation subject, not the native capture identity. Every
capture path must perform this conversion:

```text
ResourceEntry
-> CaptureTargetIdentity
-> live platform enumeration
-> unique ResolvedCaptureTarget
-> RemoteAppTargetBinding
-> native media source
```

No media stream may start from `ResourceEntry` directly.

Required internal model:

```rust
struct RemoteAppTargetBinding {
    subject_ura: String,
    target_kind: RemoteDesktopTargetKind,
    binding_id: String,
    binding_epoch: u64,
    target_identity_epoch: u64,
    target_geometry_revision: u64,
    media_source_epoch: u64,
    consent_epoch: u64,
    capture_scope: CaptureScope,
    input_scope: InputScope,
    native_locator: NativeTargetLocator,
    resolved_identity: TargetIdentity,
    geometry: TargetGeometry,
    scope_audit: ScopeAudit,
    audit_projection: serde_json::Value,
}

struct CaptureTargetIdentity {
    target_kind: RemoteDesktopTargetKind,
    discovery_backend: String,
    capture_backend: String,
    display_id: Option<u64>,
    window_id: Option<u64>,
    pid: Option<i64>,
    bundle_id: Option<String>,
    app_identity: Option<String>,
    app_name: Option<String>,
    title: Option<String>,
    bounds_snapshot: Option<TargetGeometry>,
    discovery_epoch: u64,
    freshness_deadline_ms: u64,
}

struct ResolvedCaptureTarget {
    identity: TargetIdentity,
    native_locator: NativeTargetLocator,
    match_strategy: MatchStrategy,
    confidence: MatchConfidence,
    capture_backend: String,
    display_id: Option<u64>,
    geometry: TargetGeometry,
}

enum CaptureScope {
    DisplaySurface,
    WindowSurface,
    AppSurface,
}

enum InputScope {
    ViewOnly,
    TargetLocal,
    DisplayGlobal,
}

enum TargetResolutionError {
    TargetNotFound,
    TargetStale,
    TargetMetadataIncomplete,
    TargetIdentityAmbiguous,
    TargetIdentityChanged,
    TargetIdentityMismatch,
    TargetPermissionMissing,
    UnsupportedCaptureScope,
    CaptureBackendUnavailable,
    TargetHidden,
    TargetMinimized,
    TargetDisplayUnavailable,
    TargetMultiDisplayUnsupported,
    DisplayIdentityMissing,
    DisplayIdentityMismatch,
    DisplayFallbackForbidden,
    InputScopeUnsupported,
    TransportRouteUnavailable,
    ScreenCaptureKitEnumerationFailed,
    ScreenCaptureKitFilterFailed,
    ScreenCaptureKitStreamStartFailed,
}
```

Candidate matching rules:

```text
0 candidates -> target_not_found
1 candidate  -> bind exactly and record resolved_identity
>1 candidate -> target_identity_ambiguous
```

Production candidates are built from stable native identity, not labels:

```text
window:      window_id plus owner pid/app_identity/bundle_id consistency
application: display_id plus app_identity/bundle_id/primary_pid plus resolved_window_ids/window_set_epoch
display:     monitor_id/display_id or an explicit primary_display subject
```

`app_name`, `title`, and bounds may be used only as diagnostics or
ambiguity-breaking evidence after a stable native identity candidate is already
present. They must not create a production candidate by themselves. The resolver
must not return the first matching application, window, or display when the
selector is ambiguous.

macOS v1 production matching rules:

```text
window:
  require SCWindow.windowID == metadata.window_id plus owner pid/app identity
  consistency for production capture.
  pid + app_name + title may be diagnostic only unless ambiguity is eliminated.

application:
  app_name alone is not stable identity.
  production capture requires primary_pid, bundle identity, or another stable
  native identity plus the display-scoped app window set.

display:
  require monitor_id/display_id match unless the subject explicitly means
  primary display.
```

Current macOS fallback that must be removed from production behavior:

```text
select_display(ResourceEntry) must not return firstObject() when monitor_id or
display_id is absent/mismatched.
ResourceType::Application must not call display-scoped SCK filters without an
explicit display identity or multi-stream app-surface plan.
```

Backend compatibility is part of identity resolution. A target discovered by
CoreGraphics, xcap, or another inventory backend may be captured by
ScreenCaptureKit only if the implementation provides a tested identity
translation contract for that pair. Metadata-key presence is not enough.

macOS v1 identity translation contract:

```text
CoreGraphics kCGWindowNumber -> ScreenCaptureKit SCWindow.windowID:
  must be verified in tests before being used as production identity.

window production identity:
  cg_window_id/sck_window_id
  owner_pid
  app_identity or bundle_id when available
  title snapshot
  bounds snapshot
  discovery_epoch

application production identity:
  app_identity or bundle_id when available
  primary_pid only as current-process evidence, not stable cross-restart identity
  display_id
  window_set_epoch
  resolved window ids
```

`app_name` and `title` are display labels and weak matching hints. They are not
sufficient production routing identity.

Canonical target/session failure reasons:

```text
target_not_found
target_stale
target_metadata_incomplete
target_identity_ambiguous
target_identity_changed
target_identity_mismatch
target_permission_missing
unsupported_capture_scope
capture_backend_unavailable
target_hidden
target_minimized
target_display_unavailable
target_multi_display_unsupported
display_identity_missing
display_identity_mismatch
display_fallback_forbidden
input_scope_unsupported
transport_route_unavailable
screencapturekit_enumeration_failed
screencapturekit_filter_failed
screencapturekit_stream_start_failed
```

These reason codes must be visible in `create_session` failures, lifecycle
events, or transport readiness output as appropriate. Each failure must include
one frontend recovery action:

```text
refresh_targets
request_permission
retry_session
downgrade_view_only
show_unsupported
close_session
```

The frontend cannot fix application/window capture failures if all failures collapse to `resource_unavailable`.

Error propagation contract:

```text
RemoteAppTargetError
-> Axon/ability failure reason
-> session event when a session exists
-> latest_target_diagnostic in session view
-> frontend_action
```

This mapping must be one-to-one and tested. Implementations must not recover
target reason by parsing `anyhow` messages.

The response must also expose the requested/effective scope audit:

```json
{
  "scope_audit": {
    "requested_target_kind": "window",
    "effective_target_kind": "window",
    "capture_surface": "WindowSurface",
    "input_mode": "view_only",
    "scope_widened": false,
    "display_fallback_used": false
  }
}
```

If `scope_widened` or `display_fallback_used` is true, the session must not become production-ready.

## 8. Capture scope

Legal capture scopes:

```text
DisplaySurface
WindowSurface
AppSurface
```

Mapping:

```text
display session     -> DisplaySurface
window session      -> WindowSurface
application session -> AppSurface
```

Forbidden:

- `window` or `application` session using `DisplaySurface`.
- Display capture plus crop as an equivalent implementation of window/application capture.
- Display capture plus mask/blur as an equivalent implementation of window/application capture.
- Target loss fallback to display capture.

If a platform cannot provide true app/window-scoped capture, that platform must return:

```text
unsupported_capture_scope
```

not a degraded display capture.

Application capture requires extra precision. On macOS, ScreenCaptureKit's
application filter is display-relative: it captures windows of selected
applications on a selected display. Therefore an `application` target must bind
one of these explicit forms:

```text
AppSurface(display_id, app_identity, window_set_epoch)
MultiAppSurface([AppSurface...])
Unsupported("application spans multiple displays without multi-stream support")
```

The implementation must not advertise single-stream application capture as
"the whole app" if it only captures one display's subset without saying so.

## 9. Target lifecycle events

`remote_desktop.watch_events` must include target lifecycle events, not only session/signaling/media events.

Required event types:

```text
CAPTURE_TARGET_RESOLVED
CAPTURE_TARGET_STALE
CAPTURE_TARGET_IDENTITY_MISMATCH
CAPTURE_TARGET_AMBIGUOUS
DISPLAY_FALLBACK_FORBIDDEN
SCREEN_CAPTURE_PERMISSION_DENIED
TARGET_BOUND
TARGET_MOVED
TARGET_RESIZED
TARGET_TITLE_CHANGED
TARGET_FOCUSED
TARGET_BLURRED
TARGET_HIDDEN
TARGET_VISIBLE
TARGET_MINIMIZED
TARGET_RESTORED
TARGET_LOST
TARGET_REBIND_ATTEMPTED
TARGET_REBOUND
TARGET_REBIND_FAILED
TARGET_BINDING_CHANGED
TARGET_PERMISSION_REVOKED
DISPLAY_TOPOLOGY_CHANGED
SESSION_DEGRADED
SESSION_CLOSED
```

Required event fields:

```json
{
  "event_id": 123,
  "sequence": 17,
  "session_id": "rd_...",
  "subject_ura": "easynet:///r/localhost/resource/...",
  "binding_id": "tb_...",
  "binding_epoch": 1,
  "previous_target_identity_epoch": 42,
  "target_identity_epoch": 42,
  "target_geometry_revision": 7,
  "media_source_epoch": 3,
  "transport_epoch": 9,
  "event_type": "TARGET_MOVED",
  "reason_code": "target_moved",
  "recoverability": "continue",
  "payload": {}
}
```

When a window moves or resizes, the media projection and input coordinate transform must update atomically with the event. It is not acceptable for video to update while input mapping remains stale.

Hidden/minimized/lost behavior must be explicit. Valid outcomes include pause, black frame, last-frame freeze, or termination, but never capture of unrelated desktop content.

The event stream must distinguish target failure from transport failure:

```text
TARGET_LOST: selected OS target is no longer resolvable
TARGET_PERMISSION_REVOKED: OS or EasyNet permission changed
MEDIA_SOURCE_LOST: media backend lost the bound source
TRANSPORT_FAILED: WebRTC/relay failed while target remains valid
```

Without this distinction, the frontend cannot tell "window disappeared" from "network failed".

Bounded lifecycle behavior:

```text
target tracker interval: 250-500 ms on active sessions
move/resize/title event coalescing: max 10 Hz per session
lost debounce: two failed observations or 1 second, whichever comes first
automatic rebind window: <= 30 seconds unless the user extends the lease
event log: bounded ring buffer; old target events may be compacted but not reordered
```

Long-running sessions must not accumulate unbounded target observations. A
target tracker is part of the remote desktop session lifecycle, not a global
process that owns resource identity.

Tracking driver ownership:

```text
TargetTracker starts after TARGET_BOUND and before MediaActive.
macOS v1 polls/diffs CGWindowListCopyWindowInfo and validates SCK availability.
Tracker owns the bounded observation loop and emits TargetObservation values.
Session aggregate owns committed binding state, epochs, state transitions, and
ordered event log writes.
Tracker terminates with the session.
Tracker events are ordered per session.
Media/input consume tracker snapshots, not independent OS lookups.
```

TargetTracker must not mutate `resources.json`, start media, inject input, or
directly rewrite session state. It proposes observations; the session aggregate
is the single writer of committed target lifecycle.

The session view must expose the latest target diagnostic:

```json
{
  "latest_target_diagnostic": {
    "status": "resolved",
    "reason": null,
    "requested_identity": {},
    "resolved_identity": {},
    "match_strategy": "window_id_plus_owner",
    "capture_backend": "screencapturekit",
    "display_fallback_used": false
  }
}
```

This field is required because frontend recovery depends on whether the session
failed during target resolution, native capture startup, or transport.

## 10. State machines

Remote session:

```text
PreSessionCreationWorkflow:
  ValidatingSubject
  -> AwaitingConsent
  -> ResolvingTarget

Inserted RemoteDesktopSessionAggregate:
BindingActive
-> MediaStarting
-> MediaActive
-> InputActive
-> Suspended
-> Rebinding
-> Terminating
-> Terminated
```

`AwaitingConsent` and `ResolvingTarget` are creation-workflow states, not stored
active session rows. The inserted session aggregate starts at `BindingActive`.

Target binding:

```text
Unresolved
-> Resolved
-> Stale
-> Lost
-> Rebinding
-> Resolved
-> Invalidated
```

Consent:

```text
NotRequested
-> Requested
-> Granted
-> Active
-> Revoked
-> Expired
```

Media:

```text
Detached
-> SourceAttached
-> Negotiating
-> Streaming
-> Paused
-> SourceLost
-> Stopped
```

Input:

```text
Disabled
-> AwaitingMedia
-> Enabled
-> TemporarilyBlocked
-> Disabled
```

Rules:

- Media cannot start before `BindingActive`.
- Input cannot enable before `MediaActive`, active consent, and valid target binding.
- Target loss must move the session to `Suspended`, `Rebinding`, or `Terminated`.
- Consent revocation must stop capture and input immediately.
- App/window target loss must not transition to display fallback.
- `binding_epoch` changes only when the selected OS identity changes under an explicit rebind policy.
- `target_identity_epoch` changes only when the selected OS identity changes.
- `target_geometry_revision` changes on move/resize/scale changes without changing target identity.
- `media_source_epoch` changes when the native capture source/filter is rebuilt.
- `consent_epoch` changes when the applicable EasyNet consent is granted, renewed, revoked, or replaced.
- Media frames and input coordinate transforms must refer to the same `binding_epoch` and compatible `target_geometry_revision`.

## 11. Input routing

Input dispatch must validate:

```text
session state
target binding validity
target identity epoch
input consent
session mode
target-local coordinate transform
focus/activation policy
```

For app/window sessions:

- Pointer coordinates are target-local, not display-global.
- Keyboard input must be blocked unless target focus/activation policy can guarantee dispatch to the selected target.
- If the target is hidden, minimized, lost, or no longer the input receiver, input must be disabled or temporarily blocked.

If a platform cannot guarantee target-scoped input for app/window sessions, interactive mode must fail closed or downgrade to `view_only` with an explicit reason.

macOS v1 must treat app/window keyboard injection as unsafe unless it implements
all of:

```text
foreground app/window validation before dispatch
explicit target activation policy
post-dispatch target still matches validation for bounded time
input disabled on TARGET_LOST/HIDDEN/MINIMIZED/FOCUS_CHANGED away
```

Because CGEvent-style injection is OS-global, the SPEC does not claim hard OS
sandboxing for input. It only allows interactive mode when the implementation
can prove target-directed dispatch for the current platform state; otherwise
the correct behavior is `view_only`.

## 12. Consent and permission

Separate permission scopes:

```text
OS screen recording permission
EasyNet capture consent
EasyNet input consent
clipboard consent
file transfer consent
```

Rules:

- OS screen recording permission is broad. The UI must not imply OS permission itself is scoped to one window/app.
- EasyNet consent must be scoped to the selected target subject.
- Window consent cannot authorize display session.
- Application consent cannot authorize display session.
- Capture consent cannot imply input consent.
- Capability scope increase requires new consent.
- Target identity change requires explicit rebind policy or new consent.
- Consent revocation stops media/input immediately.

## 13. Transport

Production:

```text
WebRTC RTP/SRTP for media
WebRTC data channel for pointer/key input
```

Diagnostic only:

```text
InvokeBidi
preview_stream
```

Transport state must expose:

```text
host_candidate
stun_srflx
turn_relay
easynet_relay
failed
```

No UI may report production online unless a production media path is actually ready.

## 14. Clipboard and file transfer

Clipboard and file transfer must not be extended through the pointer/key input data channel.

Future abilities:

```text
remote_desktop.clipboard.read
remote_desktop.clipboard.write
remote_desktop.clipboard.watch
remote_desktop.file_transfer.create
remote_desktop.file_transfer.accept
remote_desktop.file_transfer.send
remote_desktop.file_transfer.cancel
```

Until those abilities exist:

- Clipboard input frames must reject.
- File-drop input frames must reject.
- Capability metadata must report unsupported.

## 15. Platform strategy

### macOS v1

Inventory:

```text
CGWindowListCopyWindowInfo
NSWorkspace where bundle identity is needed
```

Capture:

```text
ScreenCaptureKit
```

Input:

```text
CGEvent
Accessibility / Input Monitoring permission
```

Tracking:

```text
poll/diff CGWindowListCopyWindowInfo
target identity epoch
ScreenCaptureKit availability checks
```

macOS minimum viable implementation order:

```text
1. refresh live CGWindowList/NSWorkspace target inventory before target picker return
2. resolve selected ResourceEntry against current SCShareableContent inside create_session
3. return target_binding with capture_scope and target_identity_epoch
4. start ScreenCaptureKit only from the resolved binding
5. emit TARGET_BOUND or typed target-resolution failure
6. add poll/diff tracker for move/resize/lost after first successful capture
```

The first implementation may mark application sessions as `capture_surface=AppSurface` and `input_mode=view_only` if target-scoped input cannot be guaranteed. It must not claim `WindowSurface` for application sessions.

macOS v1 minimum product claim:

```text
display: production when WebRTC path is ready
window: production capture only after target_binding proves exact SCWindow
application: production only when response states the exact display-scoped app
  window set; if the app spans displays and multi-stream is absent, unsupported
interactive window/application: view_only unless focus validation is implemented
```

### Linux v1

Wayland:

```text
xdg-desktop-portal ScreenCast
xdg-desktop-portal RemoteDesktop
PipeWire stream identity
restore token where available
```

X11:

```text
X11 window enumeration/capture/input
explicit weaker-security labeling
```

### Windows v1

Inventory:

```text
EnumWindows
GetWindowThreadProcessId
```

Capture:

```text
Windows Graphics Capture for window/app
Desktop Duplication only for display session
```

Input:

```text
SendInput with focus/target validation
```

RemoteApp launch reference:

```text
FreeRDP RAIL model
```

## 16. External reference boundaries

Use these as implementation references only:

- Xpra: best product-shape reference for individual graphical application remoting.
- FreeRDP RemoteApp/RAIL: Windows launched-app remoting reference.
- xdg-desktop-portal ScreenCast/RemoteDesktop: Linux permission/capture reference.
- RustDesk: capture/input/clipboard/relay engineering reference only, not target model.

Do not import RustDesk's display-level product semantics into EasyNet's resource-subject model.

## 17. Why this SPEC addresses current application/window capture failures

The implementation has partial application/window support, so the failure mode is not simply "window capture does not exist." The likely failures are at the seams:

1. **Stale target inventory.**
   Boot-time `resources.json` rows can reference a window ID, PID, title, or app aggregate that has already changed. A user can select an application/window row that is no longer resolvable by the time `create_session` or ScreenCaptureKit starts.

2. **Missing or stale target binding proof.**
   `create_session` must prove which OS window/application was bound before it inserts a session row. If ScreenCaptureKit cannot resolve the target, the frontend must see a typed `target_resolution_failed`-class error rather than a generic session or transport failure.

3. **Metadata mismatch between discovery and capture.**
   Resource bootstrap may discover via CoreGraphics or xcap metadata, while the production media path resolves via ScreenCaptureKit. If metadata keys are incomplete or not accepted by the `RemoteAppTargetResolver` / `target_for_binding` proof path, application/window capture can fail even though the target appears in `meta.list_resources`.

4. **Permission ambiguity.**
   macOS Screen Recording permission is required before ScreenCaptureKit enumeration/capture. Permission probes are host-local and must not be scoped to a display/window/application resource subject. If the frontend probes permission with the selected resource subject, it fails before capture is attempted.

5. **Incomplete target lifecycle recovery.**
   If a window closes, minimizes, moves, or app restarts, the session must emit typed target lifecycle events such as `TARGET_LOST`; full `TARGET_REBOUND` remains a later milestone unless the explicit Rebinding transition is implemented.

Executing this SPEC fixes those classes of failures only if the implementation includes all of these concrete pieces:

```text
explicit live refresh before target picker return
live target resolution inside create_session
target_binding returned in create_session response
ScreenCaptureKit target resolution typed errors
target lifecycle tracker and events
frontend remote_desktop surface that uses correct subjects
fail-closed app/window behavior with no display fallback
```

If only the surface name or descriptors are changed, the current application/window capture failures will not be solved.

### 17.1 Immediate diagnostic checklist for the current tree

Before implementing broader lifecycle tracking, the first debugging pass should prove these facts for a failing application/window:

```text
1. resource.refresh_remote_targets(types=["window","application"]) returns a fresh row for the target.
2. The row has metadata keys accepted by ScreenCaptureKit resolution:
   window: window_id plus pid, bundle_id, or app_identity
   application: display_id plus bundle_id/app_identity/primary_pid plus resolved_window_ids and window_set_epoch
3. permission_status/request_permission were invoked with user-self or descriptor-bound subject, not the target resource subject.
4. create_session receives the selected resource URA as envelope subject.
5. create_session performs live target resolution and returns target_binding.
6. ScreenCaptureKit SCShareableContent contains the selected window/application at session creation time.
7. target_for_binding failure is surfaced as target_* reason code, not generic resource_unavailable.
8. attach/WebRTC native media starts from the same resolved binding returned by create_session.
```

If any item fails, the fix is local to that seam; adding transport, relay, or frontend polish will not solve the capture failure.

### 17.2 Minimal patch sequence for app/window capture failures

The smallest implementation path that can materially fix application/window capture failures is:

```text
P0: add typed target-resolution errors and tests around ScreenCaptureKit resolution
P1: refresh window/application resources immediately before target picker/list response
P2: resolve selected target in create_session and return target_binding
P3: make native media consume the resolved binding instead of rediscovering from stale ResourceEntry
P4: emit TARGET_BOUND / TARGET_LOST / MEDIA_SOURCE_LOST with distinct reasons
P5: only after P0-P4, build full rebind and input-isolation policy
```

This order matters. Implementing `watch_events` before `target_binding`, for example, creates events about a target the session has never proven it captured.

### 17.3 Minimal cutline for the current capture bug

The first implementation milestone should not try to finish every platform and
transport feature. To directly fix "application/window cannot be captured" on
the current macOS path, the minimum cutline is:

```text
M1. resource.refresh_remote_targets refreshes current macOS windows/apps.
M2. create_session re-resolves the selected resource with ScreenCaptureKit.
M3. create_session fails with typed target errors before transport starts:
    target_not_found
    target_stale
    target_permission_missing
    unsupported_capture_scope
    target_metadata_incomplete
    target_identity_ambiguous
    display_fallback_forbidden
M4. create_session returns target_binding and scope_audit.
M5. WebRTC native media uses the exact target_binding, not a second independent
    ResourceEntry lookup that can drift.
M6. frontend target picker uses the refreshed resource URA as Invocation.subject.
```

This cutline is sufficient to turn the current generic "抓不到/黑屏/transport
failed" class into either a working app/window stream or a typed reason that
identifies the failing seam. It is not sufficient for long-running rebind,
clipboard, file transfer, TURN relay, or full interactive app/window input.

### 17.4 Required failure taxonomy

Remote desktop target failures must be typed. Generic `anyhow` text is not
enough for frontend recovery.

```text
target_not_found
target_stale
target_identity_ambiguous
target_identity_mismatch
target_metadata_incomplete
target_permission_missing
unsupported_capture_scope
capture_backend_unavailable
target_hidden
target_minimized
target_display_unavailable
target_multi_display_unsupported
display_identity_missing
display_identity_mismatch
display_fallback_forbidden
input_scope_unsupported
transport_route_unavailable
screencapturekit_enumeration_failed
screencapturekit_filter_failed
screencapturekit_stream_start_failed
```

Each failure must say whether the frontend should refresh targets, request OS
permission, retry session creation, downgrade to view-only, or show unsupported.

## 18. Required tests

Contract tests:

```text
remote_desktop.* dedicated_surface == remote_desktop
permission_status rejects display/window/application resource subjects
create_session rejects missing subject
create_session rejects subject in args
create_session requires consent causal context
create_session returns target_binding
create_session returns scope_audit
watch_events emits target lifecycle events
```

Anti-regression tests:

```text
window/application session cannot use DisplaySurface
window/application session cannot call display capture API
window/application session cannot crop display stream
target lost cannot fallback display
consent scope cannot widen
no resolved target means no media/input
clipboard/file_drop remain unsupported until split abilities exist
```

macOS target tests:

```text
resource.refresh_remote_targets does not require remote_desktop plugin state
resource refresh lists visible windows
resource refresh lists applications
meta.list_resources does not silently mutate resources.json
closed window is pruned or marked unavailable
create_session(window) binds exact window_id
create_session(application) binds explicit display-scoped app window set
application spanning multiple displays is multi-stream or unsupported
window move emits TARGET_MOVED
window resize emits TARGET_RESIZED
window close emits TARGET_LOST
app reopen emits TARGET_REBOUND only when lineage matches
```

Input tests:

```text
pointer maps to target-local bounds
pointer mapping updates after TARGET_MOVED
input rejects when target is lost
keyboard is disabled in view_only
keyboard is blocked when target focus cannot be guaranteed
```

Transport tests:

```text
WebRTC connected marks production media ready
InvokeBidi preview remains diagnostic_only
host-only candidate state is not reported as production remote-ready across NAT
TURN/relay unavailable exposes typed degraded reason
```

E2E:

```text
frontend selects a window
grant consent
create session
receive target_binding
connect WebRTC
move window -> TARGET_MOVED
resize window -> TARGET_RESIZED
close window -> TARGET_LOST
input disabled
reopen/rebind -> TARGET_REBOUND or explicit failure
no display fallback occurs
```

E2E checkpoints with authoritative evidence:

| Checkpoint | Scenario | Required evidence |
|---|---|---|
| E2E-01 target picker freshness | Open a known window after daemon boot, refresh picker, select it | `resource.refresh_remote_targets` or equivalent live inventory ran after the window existed; returned row has `availability=available`, freshness metadata, and selected `resource_ura` |
| E2E-02 permission subject correctness | Probe screen permission before selecting a target | invocation subject is user-self or descriptor-bound subject; probing with display/window/application resource subject fails with `invalid_argument` |
| E2E-03 exact window session | Select one window while unrelated bright sentinel content is visible elsewhere on the display | `create_session` returns `target_binding.resolved_identity.window_id`, `scope_audit.display_fallback_used=false`; decoded stream never includes the off-window sentinel region |
| E2E-04 exact application session | Select an app with one display-scoped window set while another app has visible sentinel content | `target_binding.capture_scope=AppSurface`; response states `display_id`, `app_identity`, `window_set_epoch`; decoded stream includes selected app windows and excludes other apps |
| E2E-05 stale window fail-closed | Select a window, close it before `create_session` | no active session row is inserted; failure reason is `target_not_found` or `target_stale`; frontend action is `refresh_targets` |
| E2E-06 no media re-resolution | Start WebRTC after successful `create_session`, then mutate/refresh resource cache before media starts | test fake `RemoteAppMediaSourceFactory` receives the stored `binding_id`; native WebRTC path has no `ResourceEntry -> target_for_entry` call outside explicit Rebinding; captured target does not drift |
| E2E-07 display fallback forbidden | Use a resource with missing/mismatched display identity or force SCK window/app filter failure | session fails with `display_identity_missing`, `display_identity_mismatch`, or `display_fallback_forbidden`; no first-display capture starts; decoded frames never show full display |
| E2E-08 move/resize tracking | Move and resize selected window during streaming | ordered `TARGET_MOVED`/`TARGET_RESIZED` events advance `target_geometry_revision`; input transform observes the same revision |
| E2E-09 target loss vs transport failure | Close selected window while WebRTC remains connected | event is `TARGET_LOST`/`MEDIA_SOURCE_LOST`, not `TRANSPORT_FAILED`; input is disabled |
| E2E-10 weak identity ambiguity | Create two windows with same app/title, select a row without stable native identity | resolver returns `target_identity_ambiguous`; no stream starts |
| E2E-11 view-only input safety | Start app/window session without proven focus-safe input routing | response reports `input_mode=view_only`; keyboard frames are rejected or ignored with `input_scope_unsupported` |
| E2E-12 frontend invocation subject | Create session from frontend selected target | Axon Invocation has selected resource URA in envelope subject and no `subject` inside args |

Passing only unit tests for resolver matching, descriptor registration, or JSON
schema does not prove the feature. M1 requires at least E2E-01 through E2E-07
and E2E-12. Full acceptance requires all E2E checkpoints above.

Performance/resource integration checkpoints:

| Checkpoint | Scenario | Required evidence |
|---|---|---|
| PERF-01 linear refresh | Fake `R=10k` persisted resources, `W=2k` windows, `A=200` apps | one platform enumeration, one indexed mutation pass, one atomic save; no `O(R*N)` upsert loop |
| PERF-02 meta read purity | Hash `resources.json`, call `meta.list_resources(types=["window","application"])` | file hash/mtime unchanged; no platform discovery invoked |
| PERF-03 shared target sampler | Run `S=128` sessions over fake `W=2k` windows | platform enumeration called once per tick, not once per session |
| PERF-04 event ring stability | Push 100k target/input/media events into one session | retained events stay at cap, sequence remains monotonic, memory stable |
| PERF-05 ICE/SDP flood bounded | Submit >10k trickle candidates and large SDP payloads | candidates rejected after cap; serialized session view remains bounded |
| PERF-06 no lock-held OS work | Instrument session-store lock while resolving/starting media | no OS enumeration, disk IO, encoder construction, SCK startup, or WebRTC negotiation occurs under the store lock |
| PERF-07 input storm coalesced | Send high-rate pointer frames and invalid frames for 10s | input path stays responsive; reject diagnostics are rate-limited/coalesced |

## 19. Acceptance definition

M1 acceptance fixes the current application/window capture failure:

```text
Frontend lists current Cursor/Safari/Chrome windows.
User selects one window or application.
Consent is scoped to that selected target.
create_session returns exact target_binding and scope_audit.
WebRTC stream shows only that target.
native media starts from the session target_binding, not ResourceEntry.
target-resolution failures return typed reasons and frontend actions.
scope_audit.scope_widened == false.
scope_audit.display_fallback_used == false.
No app/window path ever falls back to display capture.
```

Full acceptance adds independent long-running tracking and interaction:

```text
Move/resize produces TARGET_MOVED/TARGET_RESIZED and keeps input mapping correct.
Close/loss produces TARGET_LOST and disables input.
Reopen/rebind either emits TARGET_REBOUND or an explicit typed failure.
Application sessions state the exact display-scoped app window set.
Window/application interactive mode is enabled only when target-scoped dispatch is proven.
Transport readiness distinguishes host, STUN, TURN, EasyNet relay, and failed states.
```

If M1 fails, EasyNet has not solved the current app/window capture bug. If full
acceptance fails, EasyNet has not solved independent remote app/window tracking
and interaction.
