# AXON-RFC-002 — SessionProvider: kernel/backend split for persistent processes

Status: **APPROVED v2 — ready for Stage 1 implementation**
Author: Claude (with architecture from Silan)
Date: 2026-04-27
Revised: 2026-04-27 (incorporates review-pass blockers + recommendations)
Scope: cross-repo (EasyNet-Axon kernel, EasyNet-Cli daemon)
Supersedes: nothing
Depends on: AXON-RFC-001 §A16 (InvokeBidi)

Review changelog (v1 → v2):
  * §1 trait::create — `args: Value` → `args: Bytes + content_type` (B1)
  * §1 trait::attach — return `AttachResult` instead of `()` (B2)
  * §4 — `SessionRecord` gains `status: SessionStatus`, `attach_count: u32` (B5, Q2)
  * §4 — extras keys MUST be kind-prefixed (B4, Q1)
  * §4 — multi-attach semantics nailed: provider decides accept/reject;
    axon counts (B6, Q2)
  * §5 — Stage 2 fail-closed when no provider registers (B3, Q3)
  * §7 — open questions resolved by reviewer; converted to spec text

---

## §0 — Why this exists

§A3 of RFC-001 says "node-to-node = Invocation." InvokeBidi is the
streaming primitive for long-lived sessions (PTY, voice, file
transfer, pipelined LLM). §A16 nailed the wire shape (BinaryChunk,
BidiControl, HMAC frame chain).

But "session" as a runtime concept currently lives in two places:

  * **Axon** has `services/invocation/pty_attach.rs` and
    `interop_native/session_bridge.rs` — both directly spawn PTY
    children via `pty/{unix,windows}.rs`.
  * **CLI** has `daemon/execution/{session,pty,mcp_client,...}` —
    each its own service for one kind of persistent resource.

Result: **PTY is implemented twice** (axon + CLI), the boundary is
inconsistent (other persistent resources live only in CLI), and the
kernel/backend split is implicit rather than designed.

This RFC pins the split:

> Axon is the **invocation/session kernel** — it owns session URA
> allocation, InvokeBidi routing, admission, frame integrity, and a
> `SessionProvider` trait that backends register against.
>
> Backends own concrete persistent-process lifecycle — spawning a
> PTY child, holding a LLM-agent process, maintaining an outbound
> MCP connection, etc. — and register a `SessionProvider` impl with
> axon.
>
> Provider lives in the same process as axon (same daemon binary).
> Registration is static (once at daemon boot).

The mental model is **OS kernel + driver**: kernel defines the file
descriptor abstraction, drivers implement specific block devices.

This is **Model C** from the design discussion. Models A (axon owns
all backends) and B (axon is pure protocol) were rejected; see §6.

---

## §1 — Trait definition

```rust
//! In axon: `core/runtime-rs/src/services/invocation/session_provider.rs`

use bytes::Bytes;
use crate::services::invocation::bidi_handler::BidiStreamHandle;

/// Concrete-resource backend that axon dispatches `fleet.session_*`
/// calls to. One impl per persistent-resource kind (PTY, LLM agent,
/// MCP client, ...). Registered once at daemon boot via
/// `axon::session_registry::register_provider`.
///
/// `Send + Sync + 'static` because the registry holds providers
/// behind `Arc<dyn SessionProvider>` and dispatches from any tokio
/// worker.
pub trait SessionProvider: Send + Sync + 'static {
    /// Stable identifier for the resource kind this provider
    /// handles. Used by the registry to route create/attach calls.
    /// Examples: `"pty"`, `"llm:claude"`, `"mcp_client:context7"`.
    ///
    /// MUST be a stable string — registered providers cannot change
    /// their kind across daemon restarts (it's part of the
    /// session URA's audit trail).
    fn kind(&self) -> &str;

    /// Create a backend session. The session URA is **already
    /// allocated by axon** before this is called; the provider
    /// receives it and binds it to whatever backend resource the
    /// kind requires (a PTY child, an LLM process, an open MCP
    /// connection, ...).
    ///
    /// `args` is the **canonical-encoded** create payload from the
    /// wire. Axon does NOT decode it — that's the provider's job
    /// (pick the encoder that matches `content_type`: JSON for
    /// human-readable wire, CBOR/protobuf for binary efficiency).
    /// This pushes the schema to the provider where it belongs and
    /// keeps the kernel free of per-kind type knowledge.
    ///
    /// `content_type` carries the IANA media type the wire used
    /// (e.g. `"application/json"`, `"application/cbor"`,
    /// `"application/vnd.easynet.pty.create-args+protobuf"`).
    /// Provider MAY reject unsupported types with a clear error.
    /// Empty string is a protocol violation — axon enforces this
    /// at the wire layer, not in the provider.
    ///
    /// Returns `SessionMeta` carrying provider-side metadata that
    /// axon attaches to the session record (visible in the
    /// `fleet.list_sessions` response and in attach-time receipts).
    /// Failure aborts session creation; axon will not have
    /// persisted any state for `session_id`.
    fn create(
        &self,
        session_id: &str,
        args: Bytes,
        content_type: &str,
    ) -> anyhow::Result<SessionMeta>;

    /// Attach a previously-created session to a BidiStreamHandle
    /// (the InvokeBidi pipe from axon's bidi_handler). The provider
    /// is responsible for:
    ///   * pumping its backend's output into `handle.send_chunk(..)`
    ///   * receiving `handle.recv_chunk()` (or `recv_control()`)
    ///     and applying it to the backend (write bytes, resize,
    ///     send signal, eof)
    ///   * dropping the handle when the backend exits, which fires
    ///     axon's terminal-receipt path
    ///
    /// Spawned tasks SHOULD observe `handle.shutdown()` for clean
    /// teardown on connection drop. Provider does NOT emit terminal
    /// frames itself — axon's bidi_handler does that based on
    /// handle state.
    ///
    /// MUST NOT block. Long-running pumps go in `tokio::spawn`d
    /// tasks; this method returns once the pump is wired.
    ///
    /// Returns an `AttachResult`:
    ///   * `Accepted` — pump task is wired; axon emits the
    ///     in-band attach-OK receipt (`InvokeBidiDown` frame 0).
    ///   * `Rejected(reason)` — provider refused this attach
    ///     (e.g. session already attached and provider doesn't
    ///     allow multi-attach; backend in failed state). Axon
    ///     emits a Failed terminal receipt carrying the reason.
    ///     Distinct from `Err` — `Err` is a kernel-level fault
    ///     (lock poisoning, registry corruption); `Rejected` is
    ///     a provider-policy decision and stays a normal wire
    ///     response.
    fn attach(
        &self,
        session_id: &str,
        handle: BidiStreamHandle,
    ) -> anyhow::Result<AttachResult>;
}

/// Outcome of `SessionProvider::attach`. Distinguishes
/// "pump-wired-successfully" from "provider refused this attach"
/// without conflating either with kernel-level Err.
///
/// The split matters for the §A16 receipt chain: an `Accepted`
/// attach gets a fresh chain anchor (the attach-OK receipt). A
/// `Rejected` attach gets a Failed terminal receipt; no chain is
/// established, no down-direction frames emit. Without this
/// distinction, the attach-OK receipt either (a) lies about
/// admission state when the provider would later have rejected,
/// or (b) gets emitted out of order with provider readiness.
pub enum AttachResult {
    /// Provider has spawned its pump task and is ready to bridge
    /// frames. Axon proceeds to emit the in-band attach receipt
    /// and start frame routing.
    Accepted,
    /// Provider rejected this attach. The string is a human-
    /// readable reason that lands in the Failed terminal receipt's
    /// `reason` field; provider should make it actionable for an
    /// operator (e.g. `"session already attached; provider does
    /// not allow multi-attach"`, not `"nope"`).
    Rejected(String),
}

/// Per-session metadata the provider returns from `create`. Axon
/// stores this on the session record; consumers read it via
/// `fleet.list_sessions` and as receipt headers on attach.
///
/// `extras` is a kind-namespaced metadata bag. Axon does NOT
/// validate the contents at the type level — the convention is
/// enforced by code review and a CI grep, not the kernel.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionMeta {
    /// Operator-facing display name. PTY: usually the command argv0;
    /// LLM: the agent name; MCP client: the server config name.
    /// Free-form, not parsed.
    pub display: String,

    /// Provider-specific metadata. Round-trips through axon
    /// untouched.
    ///
    /// **Naming rule (REQUIRED, convention-enforced)**: every key
    /// MUST start with the provider's `kind()` value followed by
    /// an underscore. Examples:
    ///   * `pty_pid`, `pty_command`
    ///   * `llm:claude_model`, `llm:claude_session_path`
    ///   * `mcp_client:context7_transport`
    ///
    /// Why: extras from multiple providers can appear in one
    /// `fleet.list_sessions` response (different rows, but operator
    /// tools may flatten across kinds). Without the prefix two
    /// providers using `pid` would silently collide in any
    /// flatten/grep workflow.
    ///
    /// Why convention rather than kernel-enforced: validating
    /// every key on every create would couple the kernel to the
    /// provider's kind string in a way that complicates the
    /// `kind:variant` (`llm:claude`) sub-namespace pattern. A CI
    /// grep over provider impls is cheaper and catches the same
    /// bugs at PR time.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extras: serde_json::Map<String, serde_json::Value>,
}
```

### What is deliberately absent

Per Q3 decision (review pass): **no `control` method, no `close`
method.** Both are wire frames, handled by axon's bidi_handler
forwarding them onto `BidiStreamHandle` which the provider's attach
task consumes:

  * `BidiControl::PtyResize` / `PtySignal` arrives via
    `handle.recv_control()`. Provider matches and applies.
  * `BidiControl::Eof(true)` arrives the same way; provider's pump
    loop sees the down-direction signal, finishes pending I/O,
    drops its sender, lets the channel close.
  * Hard close (caller drops InvokeBidi mid-stream) closes the
    handle's channels; provider's pump detects this via `recv()`
    returning `None` and tears down the backend.

This keeps the trait small (3 methods) and pushes all
session-lifecycle communication through ONE channel (the InvokeBidi
stream). Two-channel control planes are the path to inconsistency.

### `BidiStreamHandle` signature

The handle is what bidi_handler hands the provider. Its existing
shape (already in axon) needs minor surface adjustment to be the
provider-facing API:

```rust
pub struct BidiStreamHandle {
    /// Send a binary chunk down to the caller.
    pub fn send_chunk(&self, stream_id: u32, data: Vec<u8>) -> anyhow::Result<()>;

    /// Pull the next binary chunk the caller wrote up. None = caller
    /// closed the up-direction.
    pub async fn recv_chunk(&mut self) -> Option<(u32, Vec<u8>)>;

    /// Pull the next control frame the caller sent. None = caller
    /// closed.
    pub async fn recv_control(&mut self) -> Option<BidiControl>;

    /// Cancellation token fires on connection drop / shutdown.
    /// Pump tasks should `tokio::select!` on this alongside recv.
    pub fn shutdown(&self) -> tokio_util::sync::CancellationToken;

    /// The session_id this handle is bound to (debugging /
    /// receipt-attribution).
    pub fn session_id(&self) -> &str;
}
```

Some of this exists in axon today (under different names) per
P5-rewrite-15a. RFC mandates the **public surface** above; impl
adapts internally.

---

## §2 — Registry & dispatch

### Registry

A process-wide `SessionRegistry` lives inside axon, populated at
daemon boot.

```rust
//! In axon: `core/runtime-rs/src/services/invocation/session_registry.rs`

pub struct SessionRegistry {
    providers: HashMap<String, Arc<dyn SessionProvider>>,
    sessions:  RwLock<HashMap<String /* session_id */, SessionRecord>>,
}

struct SessionRecord {
    kind:         String,        // matches some provider.kind()
    session_id:   String,        // == session URA tail
    meta:         SessionMeta,   // returned from provider.create()
    created_at:   DateTime<Utc>,
    caller:       AgentRef,      // from envelope
    subject:      SubjectRef,    // from envelope

    /// Lifecycle phase. Updated by the registry on state
    /// transitions; backs the `status` column of
    /// `fleet.list_sessions`. The provider does NOT mutate this —
    /// the registry observes provider returns + handle drops.
    status:       SessionStatus,

    /// Number of `Accepted` attaches that ever landed on this
    /// session_id. Increments inside the registry when a
    /// provider's `attach` returns `AttachResult::Accepted`.
    /// Never decrements — operator-visible "this session has
    /// been attached N times in its lifetime"; the count of
    /// CURRENTLY-LIVE attaches is the provider's business if it
    /// allows multi-attach.
    attach_count: u32,
}

/// Lifecycle phase, observed by the registry. State transitions:
///
/// ```text
///   Created ──provider.attach() returns Accepted──► Attached
///   Created ──provider.attach() returns Rejected──► Failed
///   Created ──provider.create() returned Err───────► (record never persisted)
///   Attached ──first byte flows through handle─────► Running
///   Attached ──handle drops cleanly─────────────────► Closed
///   Running  ──handle drops cleanly─────────────────► Closed
///   Running  ──handle drops with error──────────────► Failed
///   Closed   ──fleet.session_close──────────────────► Closed   (idempotent)
/// ```
///
/// Transitions are observable to a future `fleet.session_subscribe`
/// stream ability (out of scope for this RFC). For now they back
/// the static `status` field on `fleet.list_sessions` rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SessionStatus {
    /// `provider.create` succeeded; no attach has happened yet.
    Created,
    /// First `provider.attach` returned `Accepted`; pump task is
    /// wired but no bytes have flowed yet.
    Attached,
    /// Bytes have flowed in at least one direction since attach.
    Running,
    /// Backend exited cleanly OR explicit `fleet.session_close`
    /// landed. Terminal state.
    Closed,
    /// `attach` returned `Rejected` OR pump task observed a
    /// backend-level fault (PTY child crashed, LLM subprocess
    /// died with non-zero, MCP server stdio broke). Terminal
    /// state. The Failed terminal receipt carries the reason
    /// string.
    Failed,
}

impl SessionRegistry {
    pub fn new() -> Self { ... }

    /// Register one provider. MUST be called before any
    /// `fleet.session_attach` invocation can land. Panics on
    /// duplicate kind to surface configuration bugs at boot.
    pub fn register_provider(&mut self, provider: Arc<dyn SessionProvider>);

    /// Allocate a fresh session URA, dispatch to the named
    /// provider's `create`, persist the SessionRecord. Returns the
    /// new session_id (URA tail).
    pub fn create_session(
        &self,
        kind: &str,
        args: Value,
        caller: AgentRef,
        subject: SubjectRef,
    ) -> anyhow::Result<String>;

    /// Look up the provider for an existing session and dispatch
    /// `attach`. Used by the InvokeBidi handler when a
    /// `fleet.session_attach` arrives.
    pub fn attach_session(
        &self,
        session_id: &str,
        handle: BidiStreamHandle,
    ) -> anyhow::Result<()>;

    /// Snapshot of live sessions. Backs `fleet.list_sessions`.
    pub fn list_sessions(&self, filter: SessionFilter) -> Vec<SessionRecord>;
}
```

### Static registration

CLI's daemon bin (`src/bin/easynet-daemon.rs`) calls registration
exactly once during boot, before opening the IPC listener:

```rust
// In CLI: src/bin/easynet-daemon.rs (sketch)

let session_registry = axon::session_registry::SessionRegistry::new();

// Register concrete backends. Order doesn't matter; kinds must
// be unique.
session_registry.register_provider(Arc::new(
    crate::daemon::execution::pty::PtySessionProvider::new()
));
// Future:
// session_registry.register_provider(Arc::new(LlmSessionProvider::new(...)));
// session_registry.register_provider(Arc::new(McpClientSessionProvider::new(...)));

let kernel = axon::Kernel::new_with_session_registry(session_registry, ...);
```

No hot reload, no IPC-based registration. If a provider crashes at
runtime, the daemon's choice (panic vs swallow) is provider-local;
axon doesn't prescribe.

### Same-process model

Provider impls live in the same binary as axon. CLI takes
`easynet-axon` as a Rust dependency (already does). Trait dispatch
is a vtable call; no FFI, no IPC, no serialisation overhead between
axon and provider.

This rules out a future where providers are separate processes (they
can't be, today). A provider that needs to fork its own helper child
does so internally — that's a backend detail, not a kernel concern.

---

## §3 — Wire surface (operator-visible abilities)

After this RFC, the operator-visible PTY surface collapses to two
ability names that already exist in §18 of RFC-001:

| Ability | Mode | Purpose |
|---|---|---|
| `fleet.session_create` | RPC | Create a session of the named kind. Returns session_id. |
| `fleet.session_attach` | InvokeBidi | Bind an existing session to a streaming pipe. |
| `fleet.session_close` | RPC | Force-close a session (out-of-band; Eof control frame is the in-band path). |
| `fleet.list_sessions` | RPC | Snapshot live sessions across all kinds. |

`fleet.session_create` args:
```json
{
  "kind":  "pty",                    // matches provider.kind()
  "args":  { ...kind-specific...}    // forwarded to provider.create()
}
```

`fleet.session_attach` envelope's `initial_args`:
```json
{ "session_id": "<from create>" }
```

The current CLI ability names `fleet.pty_session_*` go away — they
were a kind-specific shorthand from when there was no kind dispatch.
With kind in the args, one ability set covers all backends.

Backwards compat (current `fleet.pty_session_*` callers) handled
during migration: see §5.

---

## §4 — Cross-cutting concerns

### Session URA shape

Per Q4: **axon allocates the URA**. Format follows the existing
agent-URA scheme:

```
easynet:///r/<realm>/agent/01SESS-<ulid>
```

`01SESS-` prefix distinguishes session URAs from device-profile,
hosted-agent, and llm-profile URAs in `local-agents.json`. ULID
gives time-ordered, collision-resistant ids.

The session URA is what receipts on `fleet.session_attach` carry as
`callee` — closing the audit-chain story:

  * `fleet.session_create` → receipt with `callee = device-profile
    URA`, response body = new session URA
  * `fleet.session_attach` → receipt with `callee = session URA`,
    `subject` carried from envelope, frame chain anchored on attach
    receipt's hash

This is the "session-as-subject" pattern from §A16, formalised.

### Receipt continuity across attach

Each InvokeBidi frame is HMAC-chained per §A16. The chain anchor
for a session is the `fleet.session_attach` receipt's content hash.
If the session is re-attached (a second InvokeBidi opens against
the same session_id), the new attach gets a fresh chain anchor —
the audit log shows two attach episodes with their own chains, both
referencing the same session URA.

Provider-emitted bytes are HMAC'd by axon's bidi_handler, not by
the provider. Provider just calls `handle.send_chunk(stream_id,
bytes)`; integrity is the kernel's responsibility.

### Permission / admission

Session create + attach go through the standard admission gate
(envelope verification, delegation check, trust-anchor checks, and
ability-access gates).
The provider is invoked AFTER admission passes. Failure modes:

  * Admission rejects → session never created, provider not called,
    no resource leak.
  * Provider's `create` returns Err → axon does not persist the
    session record. The URA was allocated but never published; it
    becomes unreachable garbage (acceptable, ULIDs are cheap).
  * Provider's `attach` returns Err → axon emits a Failed terminal
    receipt; the session record stays (operator can call
    `fleet.session_close` to clean up).

### Concurrency: idempotent multi-attach

Multiple `attach` calls against the same session_id:

  * **Provider decides accept-vs-reject.** Some kinds support
    multi-attach (PTY: yes — `screen`-style multiplex; LLM:
    probably no — one chat stream per session). Provider's attach
    impl returns `Accepted` or `Rejected(reason)` per its policy.
  * **Axon enforces "no destruction of in-flight attach"**: a new
    attach landing on a session that already has an active pump
    task MUST NOT cause axon to drop the existing
    `BidiStreamHandle`. The registry's role is purely to dispatch;
    the existing pump task continues to own its handle until its
    own pump exits naturally.
  * **Axon counts.** Every `Accepted` attach increments
    `SessionRecord.attach_count`. The count never decrements
    (lifetime counter, not concurrency counter); operators who want
    the live count read it from provider-emitted extras if the
    provider bothers to track them.
  * **`SessionProvider` trait has no "is this session attached?"
    query.** A provider that wants to be selective tracks its own
    state internally and uses it inside `attach` to decide
    Accepted/Rejected. Adding a query method to the trait would
    invite races (provider says "no" then yes-call lands; or vice
    versa). Better: provider's `attach` is the single decision
    point.

This is the "idempotent attach" property: re-attaching is safe
(it doesn't break in-flight work), but whether a re-attach
SUCCEEDS is the provider's call.

### Kind name collisions

Kind names are operator-facing strings used in `fleet.session_create
{ kind: "pty" }`. The registry rejects duplicate registrations at
boot to surface conflicts.

Convention: bare kind for the canonical singleton (`"pty"`,
`"llm"`), `kind:variant` for parameterised (`"llm:claude"`,
`"mcp_client:context7"`). No formal grammar; just a guideline.

---

## §5 — Migration (staged, fail-closed at Stage 2)

The current state has axon directly owning PTY (`pty_attach.rs` +
`session_bridge.rs` + `pty/{unix,windows}.rs`) and CLI also owning
PTY (`daemon/execution/pty/` + `agents/pty_*.rs`). Cleanup is
staged so `fleet.session_attach` keeps working at every commit.

### Stage 1: axon — introduce SessionProvider, keep PTY backend as opt-in

  * Land `services/invocation/session_provider.rs` (trait) and
    `session_registry.rs` (registry).
  * Wire `bidi_handler.rs`'s `fleet.session_attach` dispatch to go
    through the registry: look up by kind, dispatch to provider's
    `attach`, handle `AttachResult` accordingly.
  * Add `Kernel::seal()` step: the explicit boot point that locks
    the registry. After seal, no more `register_provider` calls
    are accepted.
  * **Adapt the existing axon PTY code into a `BuiltinPtySessionProvider`**
    behind an explicit `register_builtin_pty()` opt-in. Axon does
    NOT auto-register it. Axon's existing internal PTY tests
    explicitly call `register_builtin_pty()` from their test
    setup; nothing else does.
  * `pty_attach.rs` and `session_bridge.rs` become thin glue that
    delegates to the registry-dispatched provider. The PTY-spawn
    code (`pty/{unix,windows}.rs`) moves under
    `BuiltinPtySessionProvider` and is reachable only via the
    opt-in path.
  * Mark `BuiltinPtySessionProvider` `#[deprecated(note = "will be
    removed in Stage 3; CLI must register a PtySessionProvider")]`.

After Stage 1:
  * SessionProvider trait + registry + Kernel::seal published.
  * Axon's existing PTY tests still pass (they explicitly
    register the builtin in their setup).
  * **Axon-only daemons that previously auto-got PTY now get a
    panic at seal time** (no provider registered). This is
    intentional: the only way to get PTY is to register a provider
    explicitly.
  * No CLI changes yet — but a CLI daemon at this point is
    NON-FUNCTIONAL for PTY until Stage 2 lands. Land Stages 1+2
    in close succession (same dev session).

### Stage 2: CLI — implement PtySessionProvider, register at daemon boot (FAIL-CLOSED), deprecate `fleet.pty_session_*`

  * Move CLI's `daemon/execution/pty/` into a `PtySessionProvider`
    impl (it already has `PtyService` doing the spawning; wrap that
    behind the trait).
  * In `bin/easynet-daemon.rs`, register the CLI's
    `PtySessionProvider` BEFORE the kernel is sealed.
  * **Hard fail-closed: no silent fallback.** Stage 2 axon does
    NOT auto-register `BuiltinPtySessionProvider`. The registry's
    "panic on duplicate kind" rule from Stage 1 stays in force.
    At `Kernel::seal()`:
      * If a provider for kind `"pty"` is registered → boot proceeds.
      * If not → **panic with a clear message**:
        `"no SessionProvider registered for kind 'pty'; CLI's
        PtySessionProvider must be registered before
        Kernel::seal()"`.
    There is no env-flag escape, no fall-back. A CLI-shipped
    daemon that fails to wire its provider is a configuration bug
    and crashes loudly at boot — exactly the debugging behaviour
    we want.
  * Stage 1's `BuiltinPtySessionProvider` REMAINS in the codebase
    but is **opt-in only** during Stage 2: it's used by axon-only
    test harnesses and by axon's internal tests (which need PTY
    without dragging in CLI). Production daemon binaries link CLI
    and register CLI's provider. Stage 3 deletes it entirely.
  * Keep `fleet.pty_session_create` / `_close` / `_attach` ability
    handlers in CLI but mark them `#[deprecated]` and have them
    internally call into `fleet.session_create / _attach / _close`.
    Expose a `WARN: deprecated, use fleet.session_attach` log line
    on each call.

Rationale for hard fail-closed at Stage 2 (per Q3 review): silent
fallback is a debugging trap. An operator chasing "why is my PTY
behaviour different from what CLI's provider should give" will
take hours to discover that axon's builtin won the registration
race because of a config drift. Crashing at boot with an
unmistakable message saves all of that.

The "axon-only daemon" case the env-flag would have served is
covered by the explicit opt-in: an axon-only test harness directly
registers `BuiltinPtySessionProvider` from its own boot code. No
production daemon ever does this — production = CLI registers
PtySessionProvider. The opt-in path is for test code only and is
not advertised in any operator-facing docs.

After Stage 2:
  * CLI's PTY backend is canonical (production daemon binary).
  * Axon's builtin PTY exists but is opt-in, used only by axon's
    own internal tests.
  * Daemon refuses to boot if CLI's provider didn't register.
  * Old ability names still work as forwarders.

### Stage 3: hard cut — remove axon builtin PTY + CLI's pty_session_* aliases

  * Delete axon's `pty/{unix,windows}.rs` and
    `BuiltinPtySessionProvider`.
  * Delete axon's `interop_native/session_bridge.rs` (its callers
    migrated to `fleet.session_*` in Stage 2).
  * Restore registry's "panic on duplicate kind" — there's only one
    PTY backend now (CLI's).
  * Delete CLI's `fleet.pty_session_*` ability handlers.
  * `portable-pty` becomes the sole PTY backend dep (lives in CLI's
    Cargo.toml; axon drops its PTY-specific deps).

After Stage 3:
  * Single PTY backend (CLI), single ability surface
    (`fleet.session_*`), kind-dispatched.

### Migration test gates

Before each stage advances:

  * **Stage 1 → Stage 2 gate**: axon's existing PTY tests still
    pass against the new `BuiltinPtySessionProvider`. The §A16
    receipt-chain conformance test still passes.
  * **Stage 2 → Stage 3 gate**: a real PTY round-trip (echo
    `hello\n` through `/bin/sh`) works against CLI's
    `PtySessionProvider` over IPC. The deprecated `fleet.pty_session_*`
    aliases still forward correctly. The conformance test
    `description_for_and_input_schema_for_cover_every_published_name`
    still passes (catches a missing alias arm).
  * **Stage 3 done**: full lib test green on both repos. Conformance
    grep for `BuiltinPtySessionProvider` returns zero hits.

### Stage timeline (estimate)

  * Stage 1: 1-2 commits in axon. Pure refactor; no external API
    break.
  * Stage 2: 2-3 commits in CLI. New provider impl + daemon bin
    wiring + alias forwarders.
  * Stage 3: 1 commit each in axon and CLI, landed close together.

Total: ~6 commits, no period where PTY is broken.

---

## §6 — Models considered & rejected

### Model A — axon owns all backends

Add LLM, MCP client, discuss room, schedule, loop... all to axon.

Rejected: axon becomes a business-process container. The whole
point of the kernel/backend split is that the kernel stays small.
LLM lifecycle (subprocess management, context loaders, prompt
plumbing) has no business in a protocol kernel.

### Model B — axon stays pure protocol, CLI owns everything

axon offers no Session abstraction at all; CLI implements
`fleet.session_*` from scratch over plain InvokeBidi.

Rejected: the InvokeBidi/session-as-subject pattern from §A16
exists precisely to make session URAs first-class for federation
and audit. Throwing the abstraction away to put implementation in
CLI loses cross-node attach (Model B's CLI sessions are local-only),
loses the URA/receipt audit story, and makes every persistent-
resource backend reinvent the same machinery.

### Why Model C is the actual fit

Model C draws the line where the kernel's value adds the most:
allocation + routing + integrity + audit. Backends opt in by
implementing one trait. New persistent-resource kinds (LLM, MCP
client, ...) follow the same pattern without growing axon.

This is the conventional kernel/driver shape.

---

## §7 — Decisions (resolved by review pass)

Each row below was an open question in v1; v2 closes it. Every
decision is binding for implementation.

### Q1 — `SessionMeta.extras` collision policy
**Decision**: convention-only, kind-prefix required.
- No kernel validation.
- `extras` keys MUST start with `<provider.kind()>_`.
- A CI grep over provider impls catches violations at PR time.
- See §1 `SessionMeta.extras` doc-comment for examples.

### Q2 — multi-attach
**Decision**: provider decides accept-vs-reject; axon counts.
- Provider's `attach` returns `Accepted` or `Rejected(reason)`.
- Axon increments `SessionRecord.attach_count` on every
  `Accepted` (lifetime counter, never decrements).
- Axon does NOT enforce single-attach.
- Axon does NOT destroy in-flight attaches when a new attach
  lands (see §4 "Concurrency: idempotent multi-attach").
- No "is-attached?" query on the trait — provider tracks its
  own state if it cares.

### Q3 — duplicate-kind / fallback
**Decision**: hard fail-closed at Stage 2; no silent fallback ever.
- Stage 1: axon's `BuiltinPtySessionProvider` exists but is
  opt-in only (test harnesses register it explicitly).
- Stage 2: CLI's `PtySessionProvider` is mandatory in any
  CLI-shipped daemon. Missing registration → panic at
  `Kernel::seal()` with a clear actionable message.
- No env-flag escape. Configuration drift is a boot-time crash,
  not a runtime mystery.
- See §5 Stage 2 for the panic message and rationale.

### Q4 — future kinds and RFC obligations
**Decision**: reuse this RFC; no per-kind RFC required.
- The `SessionProvider` contract is kind-agnostic.
- A new kind (LLM agent session, MCP client connection, voice
  session, etc.) just implements the trait and gets registered.
- Kind-specific args/extras schema is documented in the
  provider's own module-level docs, not in a new RFC.
- The CI grep that enforces extras kind-prefix scales naturally.

### Q5 — re-attach receipt chain
**Decision**: not a provider concern; query the receipt log.
- Each `Accepted` attach establishes its own §A16 frame chain
  anchored on its attach receipt's content hash.
- "Walking previous attach episodes for a session_id" is a
  query over the receipt log: `WHERE callee == session_uri`.
- The provider does not maintain attach history; axon does
  not maintain attach history; the receipt log is the source
  of truth.

---

## §8 — Approval

**APPROVED v2 (2026-04-27)** — implementation may start at Stage 1
(axon).

The v1 draft had three blockers (B1-B3) and three recommendations
(B4-B6); all are folded into v2 above. The five §7 questions are
resolved; their decisions are binding.

Implementation order is the staged migration in §5: Stage 1
(axon trait + registry + opt-in builtin), then Stage 2 (CLI
provider + fail-closed boot + ability rename), then Stage 3 (delete
builtin + delete CLI's deprecated aliases). Stages 1 and 2 should
land in close succession to avoid leaving a broken-PTY window.

Authority to edit EasyNet-Axon for Stage 1 is granted to Claude
per the review-pass instruction "B: You implement the Axon side."
