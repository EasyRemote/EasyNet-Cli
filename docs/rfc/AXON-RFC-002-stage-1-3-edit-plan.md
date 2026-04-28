# AXON-RFC-002 Stage 1.3 — concrete edit plan for bidi_handler refactor

Status: **awaiting reviewer approval before implementation**
Date: 2026-04-27

This document spells out the exact code edits Stage 1.3 will land,
broken into the four commits that follow Step 1 (`73ea770` —
AxonState wiring, already done).

The motivation for writing this plan rather than diving in: the
bidi_handler frame loop (`run_frame_loop`, ~280 lines) is the
hot-path code that every InvokeBidi call goes through. Getting
its refactor wrong silently breaks PTY for everyone. A pre-commit
plan lets you catch design errors before they're embedded in
five interleaved commits.

The plan respects every hard constraint from the Stage 1.3
greenlight:

  - bidi_handler remains the sole owner of: sequence, HMAC chain,
    frame encoding, receipt emission
  - providers must never construct frames or signatures
  - attach() only wires backend to BidiStreamHandle, does not
    drive protocol flow
  - ProviderExit only triggers terminal receipt via bidi_handler
  - remove all hardcoded PTY branches; routing must go through
    registry

---

## Step 2: Registry dispatch replacement

**Files**: `bidi_handler.rs` (one section).

**Edit**: replace the `if ctx.ability_name == ABILITY_PTY_ATTACH
{ pty_ability = Some(bind_pty_attach(...)?) }` block (lines
541-562) with a `dispatch_attach()` call that:

  1. Looks up the registry on `ctx.runtime.state.session_registry`.
  2. Calls `session_registry.attach_session(session_id, handle)`.
  3. Returns the attached `BidiStreamHandle`'s internal halves
     so the frame loop can pump them.

**Open question**: where does `session_id` come from? Today
`pty_attach::bind_pty_attach()` parses it from `ctx.initial_args`
(JSON: `{"session_id":"..."}`). Two options:

  * **Option A**: bidi_handler keeps parsing `initial_args` to
    extract `session_id` (single field, JSON-decoded), then
    passes it to the registry. Provider's `create()` got called
    earlier through a *separate* `fleet.session_create` ability.
  * **Option B**: bidi_handler parses ONLY `session_id` and
    `kind` from `initial_args`, then calls
    `session_registry.create_session(...)` IF the session_id is
    new, then `attach_session(...)`. Combined create+attach.

The RFC §3 wire surface lists `fleet.session_create` (RPC) and
`fleet.session_attach` (InvokeBidi) as separate abilities.
**Option A** matches the RFC; Option B would conflate them.

**Plan**: Option A. session_create is a future unary ability
landed in Stage 2 (CLI side); for Stage 1.3 the existing PTY
flow that's already create-then-attach-by-id gets preserved —
session_bridge.session_start (unary) is what creates the session
today, then `fleet.session_attach` (bidi) attaches by id. The
registry doesn't see the create call until Stage 2 wires
`fleet.session_create` to it; in the meantime, the registry's
session table is populated by a temporary "synthesise a record
on first attach" path (documented as deprecated in code).

Actually this is gnarly. Let me revise:

**Plan (revised)**: in Stage 1.3 Step 2, keep the existing
session creation path (session_bridge.session_start is unary,
runs through the existing handler). For attach, the registry
needs to know which kind to dispatch to. Today the `initial_args`
JSON has only `session_id`; we look up the existing session in
the session_bridge state to find its kind (always "pty" today),
then dispatch through the registry.

**Edit**:

```rust
// Replace this block in run_frame_loop (lines 541-562):
let mut pty_ability: Option<super::pty_attach::PtyAbility> = None;
if ctx.ability_name == super::pty_attach::ABILITY_PTY_ATTACH {
    let node_id = if ctx.target_node_id.is_empty() { ... };
    match super::pty_attach::bind_pty_attach(...) {
        Ok(ab) => pty_ability = Some(ab),
        Err(err) => return LoopOutcome::fail(err.to_string(), ...),
    }
}

// With:
let session_handles =
    if ctx.ability_name == super::pty_attach::ABILITY_PTY_ATTACH {
        match dispatch_attach_via_registry(ctx, &down_tx).await {
            Ok(handles) => Some(handles),
            Err(err) => return LoopOutcome::fail(err.to_string(), down_seq, last_down_mac),
        }
    } else {
        None
    };
```

Where `dispatch_attach_via_registry` is a new private function in
`bidi_handler.rs` that:

  1. Parses `session_id` from `ctx.initial_args` (JSON).
  2. Looks up the SessionRecord via
     `ctx.runtime.state.session_registry.get_session(session_id)`.
     Missing session → return error.
  3. Builds the four-channel set the registry's `attach_session`
     needs (the same channels `BidiStreamHandle::new_for_test`
     produces, but for production: bidi_handler owns the OTHER
     ends, just like it owns the up/down sides today).
  4. Calls `session_registry.attach_session(session_id, handle)`.
  5. Inspects `AttachResult` — `Rejected { reason }` → return
     error string for terminal receipt; `Accepted` → return the
     opposite-end channels (`down_rx`, `up_chunk_tx`,
     `up_control_tx`, `exit_rx`) that the frame loop will pump.

The pump part is Step 3.

**Test**: existing `bidi_handler` tests pass. PTY round-trip
test (`test_pty_attach_*`) still works because the registry is
seeded with the BuiltinPtySessionProvider (Step 2 also adds the
provider impl).

**Provider impl in Step 2**: a new file
`services/invocation/builtin_pty_provider.rs` implements
`SessionProvider` for kind="pty" by calling the existing
`pty_attach::bind_pty_attach()` internally. Its `attach()`:

```rust
fn attach(&self, session_id: &str, handle: BidiStreamHandle)
    -> anyhow::Result<AttachResult>
{
    // Parse the session_id, call bind_pty_attach with the handle's
    // peer-side channels in a spawned task. Return Accepted.
    // Backend exit drops the handle's channels, which the frame
    // loop in bidi_handler observes as channel-closed → terminal.
    todo!()
}
```

`create()` is mostly empty for now — sessions are still created
through `session_bridge.session_start`. A future commit migrates
that.

The boot path: AxonState constructs `session_registry` unsealed.
Stage 1.3 Step 2 ALSO adds an opt-in registration call in the
test setup that calls
`registry.register_provider(Arc::new(BuiltinPtySessionProvider::new(state.clone())))`
then `registry.seal()`. Production daemon doesn't get this yet —
it lands in Stage 1.5 (panic-if-no-PTY-provider).

**Wait — this means Stage 1.3 Step 2 BREAKS production PTY** if
the daemon doesn't auto-register the builtin. Two mitigations:

  * **Mitigation a**: Step 2 includes a TEMPORARY auto-register
    of BuiltinPtySessionProvider in `AxonRuntime::new`, marked
    `#[deprecated(note = "Stage 1.5 removes auto-register")]`.
    Production keeps working; tests still pass.
  * **Mitigation b**: Step 2 ALSO lands the seal-time-check
    that's planned for Stage 1.5, but with a warn-then-fall-back
    instead of panic for now. Stage 1.5 flips it to panic.

**Plan**: Mitigation a. Auto-register the builtin during Stages
1.3-1.4; Stage 1.5 removes the auto-register and adds the panic
check. This lets each commit be independently green AND keeps
production PTY working at every commit.

---

## Step 3: BidiStreamHandle channel integration

This is part of Step 2 actually — there's no clean split. The
moment we wire registry dispatch (Step 2) we have to thread the
real channels through. The dual-frame-loop (existing PTY
ability vs new registry dispatch) approach would be too much code
churn.

**Combined Step 2+3 commit**: registry dispatch with real channel
integration AND the BuiltinPtySessionProvider that wraps the
existing `bind_pty_attach`. Old `pty_ability: Option<PtyAbility>`
local in `run_frame_loop` becomes `session_handles: Option<...>`;
the dispatch arms (`FrameEvent::Up(BinaryChunk)`,
`Control(PtyResize)`, etc.) push through the new
`session_handles`'s sender channels instead of calling
`pty.on_chunk()` directly.

bidi_handler's frame loop then has these arms:

```rust
match frame.payload {
    Some(BinaryChunk(chunk)) => {
        if let Some(ref handles) = session_handles {
            // Push to the up_chunk_tx channel.
            // Provider's pump task reads from up_chunk_rx.
            handles.up_chunk_tx.send(...).await?;
        } else {
            // v1 echo path stays for non-session abilities.
            emit_binary_down(...).await;
        }
    }
    Some(Control(PtyResize/PtySignal/Eof)) => {
        if let Some(ref handles) = session_handles {
            handles.up_control_tx.send(...).await?;
        }
    }
    ...
}
```

The down-direction pump (PTY → wire BinaryChunks) gets fed from
`session_handles.down_rx` (provider writes into the down_tx clone
the BidiStreamHandle holds; bidi_handler reads from the rx end and
emits framed down chunks via the existing `emit_binary_down()`).

This means the loop body grows a third FrameEvent arm:
`FrameEvent::SessionDown(InboundChunk)`, replacing `FrameEvent::PtyOutput`.

**Test gate**: the existing PTY round-trip test must still pass.
This is the canary for "did I break bidi_handler?"

---

## Step 4: ProviderExit → terminal receipt

The exit one-shot lives on `BidiStreamHandle` already (a field
that bidi_handler will keep the rx end of, opposite to the
provider's tx end).

**Edit**: add a fourth `FrameEvent` arm:
`FrameEvent::ProviderExit(ProviderExit)`. When the provider's
pump task drops the handle (clean exit) or sends an explicit
`ProviderExit`, the rx fires; the frame loop turns the
`ProviderExit::Completed` / `Failed(reason)` into
`LoopOutcome::complete` / `fail` and exits.

**Test**: a new bidi_handler test that registers a provider
which immediately calls the equivalent of "drop the handle" →
verify Completed terminal receipt fires with correct down_seq +
chain.

---

## Step 5: remove legacy PTY path

After Steps 2+3+4 land, `pty_attach.rs`'s `PtyAbility` and
`bind_pty_attach()` are unused (the BuiltinPtySessionProvider
wraps the underlying `pty_*_bytes` helpers from session_bridge
directly, NOT through PtyAbility). Delete:

  * `pty_attach.rs` body — keep just the `ABILITY_PTY_ATTACH`
    constant string (other code references it as the ability
    name); remove `PtyAbility`, `bind_pty_attach`,
    `PtyExitReason`, all the binding glue.
  * `bidi_handler.rs`'s `FrameEvent::PtyOutput` and
    `FrameEvent::PtyClosed` arms (replaced by `SessionDown` and
    `ProviderExit`).

**Test**: full PTY round-trip still passes, conformance grep for
`pty_attach::PtyAbility` returns zero hits (outside the constant).

---

## Combined commit count

Per the original "5 suggested commits" goal:

  1. ✅ `73ea770` — AxonState wiring (done)
  2. **Combined Step 2+3** — registry dispatch + channel
     integration + BuiltinPtySessionProvider + auto-register
     (kept for migration). One commit because the dispatch path
     is unsplittable from the channel wiring.
  3. **Step 4** — ProviderExit → terminal receipt arm
  4. **Step 5** — delete legacy PTY path
  5. (Stage 1.4) — opt-in `register_builtin_pty()` removes the
     auto-register; Stage 1.5 adds the panic-if-no-PTY-provider
     seal-time check.

So Stage 1.3 lands as 3 more commits (4 total). Stage 1.4 is
still its own commit; Stage 1.5 is its own commit. Total Stage 1
commit count: 7.

---

## Things I will NOT change in Stage 1.3

  * Sequence / HMAC chain / frame encoding semantics in
    bidi_handler. These stay byte-identical. The §A16 conformance
    test is the regression guard.
  * The `emit_binary_down`, `build_down_frame`,
    `emit_terminal_receipt` helpers. Providers do NOT call these;
    bidi_handler does. The handle's `send_chunk(stream_id, bytes)`
    enqueues into a channel; bidi_handler's loop reads the channel
    and calls `emit_binary_down` itself. Frame construction stays
    in one place.
  * Admission gate, delegation gate, membership gate. Untouched.
  * Receipt schema. Untouched.

---

## Reviewer asks

Three questions I want explicit answers on before I land Step 2+3:

  1. **OK to combine Step 2 and Step 3 into one commit?** They
     can't be separated cleanly without a non-buildable
     intermediate state. Original "5 commits" suggestion was
     conceptual; combining 2+3 keeps every commit independently
     green.

  2. **OK with auto-register migration tactic?** Stage 1.3 keeps
     production PTY working by auto-registering
     `BuiltinPtySessionProvider` inside `AxonRuntime::new`,
     marked deprecated. Stage 1.5 flips to fail-closed (per RFC
     §5).

  3. **OK with Step 5 deleting `PtyAbility`/`bind_pty_attach`
     entirely?** The BuiltinPtySessionProvider talks to
     `session_bridge`'s `pty_*_bytes` helpers directly, bypassing
     PtyAbility's thin wrapper. Deleting `pty_attach.rs` body
     (keeping just the ABILITY constant) is the cleanup.

If yes to all three, I proceed. If you want a different
sequencing, say so before I sink the bidi_handler edits.
