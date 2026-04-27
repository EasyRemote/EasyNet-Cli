# AXON-RFC-002 Stage 2 — Topology Addendum

**Status**: Blocking. Stage 2 cannot proceed as originally written.
This addendum documents the topology mismatch discovered during
Stage 1 close-out and proposes two paths forward for user decision.

**Date**: 2026-04-27
**Author**: Claude (with Silan.Hu architectural authority)
**Supersedes**: §5 Stage 2 of `AXON-RFC-002-session-provider.md`
(only the Stage 2 portion; Stage 1 and Stage 3 framing stand)

---

## 1. The mismatch

The original RFC Stage 2 says:

> In `bin/easynet-daemon.rs`, register the CLI's `PtySessionProvider`
> BEFORE the kernel is sealed.

This assumes CLI and Axon run in the **same process** with CLI calling
`SessionRegistry::register_provider(...)` against an in-process
`AxonRuntime`.

Reality on inspection:

- **Axon-runtime** is its own binary in `EasyNet-Axon/core/runtime-rs/`.
  It owns the `SessionRegistry`, `BidiStreamHandle` channel quartet,
  and the in-process `SessionProvider` trait dispatch.
- **EasyNet-Cli daemon** (`easynet-daemon`) is a separate process.
  It owns the `LocalAbilityRegistry`, `PtyService` (portable-pty
  spawning), and its own `register_bidi` IPC surface for PTY
  abilities.
- CLI consumes Axon only as a Rust SDK (`easynet-axon = path =
  "../EasyNet-Axon/sdk/rust"`) which exposes `DendriteBridge`,
  `AxonError`, MCP scaffolding, ability deploy helpers — **not**
  `SessionProvider`, `SessionRegistry`, or `BidiStreamHandle`.

**CLI cannot call `register_provider` because the registry is in
another process.** The trait shape (`fn create(&self, ...) ->
SessionMeta`) requires synchronous in-process dispatch.

## 2. Why this didn't show up earlier

Stage 1 ran entirely inside `core/runtime-rs/` and the
`BuiltinPtySessionProvider` shim auto-registered from
`AxonRuntime::new`. Inside Axon's process this works fine and the
new `fleet.session_attach` path goes registry → BuiltinPty →
`session_bridge::pty_*_bytes` → existing axon PTY backend.

Today's PTY traffic in production uses CLI's `fleet.pty_session_*`
abilities (registered against CLI's `LocalAbilityRegistry`),
**not** the Axon `fleet.session_attach` path. The two paths exist
side by side; Axon's auto-registered builtin is currently
unreachable in production because nobody dispatches
`fleet.session_attach` against the Axon-runtime process.

This is consistent with the RFC's Stage 1 framing ("non-functional
for PTY until Stage 2 lands") — Stage 1 was always going to need
Stage 2 to make the new path live.

## 3. Two paths forward

### Path A — Move PTY hosting into Axon process

**Change**: Move CLI's `runtime/execution/pty/` into Axon as a
first-class provider. CLI's `fleet.pty_session_*` ability handlers
become thin RPC forwarders to Axon's `fleet.session_*` over the
existing Axon SDK.

- **Pro**: Matches the RFC's original "single PTY backend" goal.
- **Pro**: Stage 3 cleanup is straightforward (delete CLI's PTY
  module, keep ability-name shim).
- **Con**: Moves `portable-pty` from CLI's deps into Axon's. The
  RFC explicitly puts portable-pty in CLI (§5 Stage 3, last
  bullet). This contradicts the RFC's stated end state.
- **Con**: Axon-runtime now owns interactive-shell spawning, which
  was deliberately a CLI-shaped concern (operator's machine, not
  shared kernel infra).
- **Con**: Bigger blast radius — touches both repos in lockstep.

### Path B — IPC-bridged provider registration

**Change**: Axon SDK exposes a `RemoteSessionProvider` shim that
the CLI registers locally. The shim is a thin client that talks to
Axon-runtime over IPC; Axon's `SessionRegistry::register_provider`
gets a remote-provider variant that knows it dispatches over the
network.

- **Pro**: Matches the existing two-process topology. CLI keeps
  PTY ownership; Axon delegates kind-dispatch to wherever the
  provider physically lives.
- **Pro**: Establishes the pattern for future cross-process
  providers (a remote LLM agent host, a sidecar MCP fleet).
- **Con**: The `SessionProvider` trait surface needs an async
  variant or a wrapper; the current `fn create(...) ->
  anyhow::Result<SessionMeta>` is sync-blocking and can't be the
  cross-process boundary as-is.
- **Con**: Doubles the wire-protocol surface (registry RPC for
  create/attach in addition to BidiStreamHandle's frame plumbing).
- **Con**: Bigger Axon-side delta than Path A. RFC Stage 2 budget
  was 2-3 commits; Path B is more like 6-8.

### Path C — Defer to a unified-binary world

**Change**: Acknowledge that Stage 2 was written for a
hypothetical merged-binary `easynet` that owns both Axon and CLI
abilities in one process. Mark Stage 2 as blocked pending that
merge; freeze the current parallel paths in place.

- **Pro**: Zero new code; explicit about scope.
- **Pro**: Lets us land genuinely useful work elsewhere (Backend
  M5/M5b/M5c, hub federation.subscribe_directory) instead of
  building a Stage 2 that may not survive a future binary merge.
- **Con**: Stage 1's `BuiltinPtySessionProvider` stays
  technically-unreachable forever (or until merge happens). It's
  exercised only by Axon's internal tests.
- **Con**: Two PTY paths persist (CLI's `fleet.pty_session_*` for
  production, Axon's `fleet.session_*` for tests).

## 4. Recommendation

**Path C** for the next iteration. Reasoning:

1. The RFC's stated end goal — one PTY backend, one ability
   surface, kind-dispatched — has real value, but only if CLI and
   Axon are eventually one process. If they stay separate forever,
   Path B's complexity buys us little.
2. Stage 1's deliverables (registry, trait, channel-close
   semantics, builtin shim) **did** standardise the kernel
   interface and produce reusable Axon-internal infrastructure.
   Even unused in production, it is the right shape for whichever
   future path lands.
3. The remaining task queue has higher-value work that is
   blocked on nothing: Backend `wshandler.go` migration to
   InvokeBidi (#122), SSE on InvokeStream (#127), federation
   `subscribe_directory` (#152). These move user-visible needles.

**If Path A or B is preferred**, the next step is a 2-3 day
design pass producing a Stage-2-revised RFC that picks the path
explicitly. This addendum is a holding document until that
decision lands.

## 5. Status of related tasks

- Task #183 RFC-002 Stage 1 (Axon): **complete**. The trait,
  registry, channel-close semantics, and BuiltinPtySessionProvider
  shim all landed in `EasyNet-Axon/core/runtime-rs/`. 250/250
  tests passing.
- Task #184 RFC-002 Stage 2 (CLI): **blocked on this addendum**.
  Cannot proceed without picking Path A, B, or C.
- Task #185 RFC-002 Stage 3 (cross-repo): **blocked on Stage 2**.

## 6. What Stage 1 left in place

For the record, here is the production state after Stage 1:

| Layer | What runs | Where it lives |
|---|---|---|
| CLI PTY abilities | `fleet.pty_session_create / _close / _attach` against `LocalAbilityRegistry` | `EasyNet-Cli/src/runtime/agents/pty_*_ability.rs` + `runtime/execution/pty/` |
| Axon PTY plumbing | `BuiltinPtySessionProvider` auto-registered into `SessionRegistry`; serves `fleet.session_attach` if ever called | `EasyNet-Axon/core/runtime-rs/src/services/invocation/builtin_pty_provider.rs` |
| Wire dispatch (today) | Backend → CLI daemon over CLI IPC; PTY traffic uses `fleet.pty_session_attach` (BIDI) | unchanged |
| Wire dispatch (post-Stage-2) | Backend → Axon daemon over Axon RPC; PTY traffic uses `fleet.session_attach` (BIDI) → `SessionRegistry` → Path-A/B-dependent backend | depends on path chosen |

The Stage 1 work is **not wasted** — it is the necessary foundation
for any future path. It just isn't reachable from production
traffic today.

---

End of addendum.
