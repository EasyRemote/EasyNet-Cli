# C-M3a Design — `LocalAbilityRegistry.register_bidi` + IPC InvokeBidi routing

Status: **DRAFT — review before code**
Author: Silan Hu
Date: 2026-04-27
Scope: CLI only. Backend `RealClient.InvokeBidi` already exists; Axon FFI already exposes the bidi quartet.

---

## Goal

Add the third call mode to the CLI's local ability registry + IPC control plane:

| Call mode  | Registry method        | IPC inbound frame                          | IPC outbound frames                              | Status today |
|------------|------------------------|--------------------------------------------|--------------------------------------------------|--------------|
| RPC        | `register_rpc`         | `Invoke{request_id, ability, args}`        | one `Result` / `Error`                           | done |
| Stream     | `register_stream`      | `Subscribe / Cancel`                       | N `Frame` + `Terminal` / `Error`                 | done |
| **Bidi**   | **`register_bidi`**    | **`OpenBidi / SendBidi / CloseBidi`**      | **N `RecvBidi` + `Terminal` / `Error`**          | **this PR** |

The bidi handler model is the natural extension: a long-lived session where both sides may push frames at any time until either closes.

---

## Out of scope

- Backend wshandler (C-M5).
- PTY (C-M3b/c) — built **on top** of this infra.
- Remote bidi forwarding through `GatewayApi` — local-only in C-M3a, mirrors how stream remote-forwarding was deferred in PR-SYS.
- Rewriting `Subscribe` over bidi. Stream stays as-is.

---

## Design decisions (the gated questions)

### D1. `BidiSource` representation

**Decision**: a struct holding **two `tokio::sync::mpsc` channels** — one for client→handler frames, one for handler→client frames.

```rust
pub struct BidiSource {
    /// Frames from client to handler. Handler reads.
    pub from_client: mpsc::Receiver<Value>,
    /// Frames from handler to client. Handler writes.
    pub to_client: mpsc::Sender<Value>,
}
```

**Rejected**: `broadcast` channels (used by Stream Live mode) — bidi is point-to-point per session, fan-out is wrong; broadcast's lag-on-slow-consumer semantics make every dropped frame an error rather than backpressure.

**Rejected**: a single duplex channel — would require encoding direction in the payload, defeats the type-system distinction.

**Bound**: 256 frames each direction (matches the IPC writer queue — same backpressure budget). Configurable later if a specific ability needs more.

### D2. Handler shape: tokio task vs sync loop

**Decision**: handlers return `BidiSource` immediately; the **handler closure spawns its own tokio task** that owns the long-lived loop. The dispatcher never blocks waiting for a handler's loop body.

```rust
pub type LocalBidiHandler =
    Arc<dyn Fn(Value) -> anyhow::Result<BidiSource> + Send + Sync>;
```

The handler closure pattern: build the two channels, spawn `tokio::spawn(async move { ... handler loop reads from rx, writes to tx ... })`, return `BidiSource{from_client: client_to_handler_rx, to_client: handler_to_client_tx}`.

**Why**: matches how `register_stream`'s `Live` variant works — the handler returns an already-running source, the IPC layer is a pure forwarder. Keeps the registry abstraction transport-agnostic and lets the IPC server treat all handler types uniformly (snapshot, live stream, bidi) with the same forwarder primitive.

**Rejected**: registry hands the handler an `async fn` taking the channels — would force every handler to be `'static + Send + Sync` async fn, harder to compose with services that hold non-`Sync` state.

### D3. Forwarder backpressure

**Decision**: `mpsc::send().await` semantics — natural backpressure. If the IPC writer queue is full, the bidi forwarder awaits; if the handler is slow, `SendBidi` frames pile up in `from_client` until the bound is reached, at which point the IPC server's read loop blocks awaiting `from_client.send().await`. The connection-level `out_tx` (256-frame bound) provides the global per-connection cap.

**No** "drop oldest" or "drop newest". Drop semantics belong to specific abilities (e.g. video frames), implemented in the handler, not in the transport.

### D4. Cancel semantics

**Decision**: three cancel paths, all converge on the same teardown:

1. **Client-initiated (`CloseBidi`)**: client drops or sends `CloseBidi{session_id}`. IPC server drops the `from_client` sender → handler's `recv()` returns `None` → handler exits its loop → `to_client` sender drops → IPC forwarder sees `None` → emits `Terminal{session_id, reason:"done"}`.

2. **Handler-initiated**: handler decides to end (e.g. PTY child exits). Handler drops `to_client` → IPC forwarder sees `None` → emits `Terminal{session_id, reason:"done"}`. Pending client frames buffered in `from_client` are dropped.

3. **Connection drop**: serve_connection's reader loop ends → `cancel.cancel()` for every registered session → IPC forwarder observes the token → emits `Terminal{cancelled}` and drops the channels.

A `CancellationToken` per session is registered in the same `CancelRegistry` that streams use. **No new registry type**.

### D5. Per-frame error path

**Decision**: ability-level errors travel as `Error{session_id: Some(...), code, message}` envelopes interleaved with `RecvBidi` frames. Receiving an `Error` does NOT close the session — the handler decides whether to continue or to drop `to_client`. The latter triggers a `Terminal` per D4 path 2.

**Why**: a long-running PTY session can have transient command failures that don't kill the session. A backend SSE stream might want a per-event error without unsubscribing.

**`Terminal` reasons**: `done` (clean), `cancelled` (any cancel path), `error:<short_code>` (handler raised a fatal error before close).

### D6. Wire frame additions

```rust
// IncomingFrame additions:
OpenBidi  { session_id: String, ability: String, #[serde(default)] args: Value }
SendBidi  { session_id: String, frame: Value }
CloseBidi { session_id: String }

// OutgoingFrame additions:
RecvBidi  { session_id: String, frame: Value }
// Terminal + Error are reused (already populated for stream).
```

**No `Cancel` reuse**: `Cancel{subscription_id}` could route a cancel for a bidi session if `subscription_id == session_id`, but the wire variant name lies. Adding `CloseBidi` is cheaper than confusing future readers.

### D7. Registry API

```rust
pub fn register_bidi(&mut self, ability: impl Into<String>, handler: LocalBidiHandler);
pub fn get_bidi(&self, ability: &str) -> Option<&LocalBidiHandler>;
```

`list_abilities()` extends to include bidi keys (union of rpc + stream + bidi).

`CallMode::Bidi` added to the enum. `AbilityDispatcher::execute_bidi(target) -> anyhow::Result<BidiSource>` parallels `execute_rpc / execute_stream`.

### D8. ability_proxy wiring

`handle_async` gains three arms: `OpenBidi → handle_bidi_open_async`, `SendBidi → route to session's from_client tx`, `CloseBidi → drop session's from_client tx + cancel token`.

A new per-connection `BidiRegistry: HashMap<session_id, mpsc::Sender<Value>>` lives next to `CancelRegistry` so `SendBidi` frames can find the right handler-input channel. Registry is dropped when the connection closes (every session's `from_client` sender closes simultaneously, every handler exits, every forwarder emits its `Terminal`).

---

## Test plan

Inside `ability_dispatch.rs` and `services/control/`:

1. **registration test** — `register_bidi` makes the ability dispatchable.
2. **echo handler round-trip** — send 3 frames, observe 3 echoed `RecvBidi` frames.
3. **client close** — drop client side mid-stream, observe `Terminal{done}`.
4. **handler close** — handler drops `to_client` after 2 frames, observe `Terminal{done}` after 2 RecvBidi frames.
5. **cancel-on-connection-drop** — drop the underlying UnixStream, observe `Terminal{cancelled}`.
6. **per-frame error** — handler emits an `Error` envelope, session continues, then closes cleanly.
7. **wire shape** — `OpenBidi/SendBidi/CloseBidi/RecvBidi` JSON round-trip.

End-to-end smoke (extends the existing `end_to_end_invoke_round_trip_returns_result_for_system_ping`) using a registered echo bidi handler.

---

## Estimated scope

| File                                          | Δ LOC (incl. tests + comments) |
|-----------------------------------------------|-------------------------------:|
| `src/runtime/ability_dispatch.rs`             | +120 |
| `src/runtime/invocation_target.rs`            |  +10 (CallMode::Bidi) |
| `src/services/control/frames.rs`              |  +60 |
| `src/services/control/ability_proxy.rs`       | +180 |
| `src/services/control/server.rs`              |  +20 (BidiRegistry plumbing) |
| Echo bidi handler + tests in two of above     | +150 |
| **Total**                                     | **~540** |

Matches the task's "≥500 LOC" estimate.

---

## Open questions for review

1. **D1 channel bound**: is 256 the right default? PTY scrollback could push much higher.
2. **D5 error not closing session**: agree this matches PTY/SSE needs?
3. **D8 BidiRegistry placement**: per-connection (proposed) or per-process? Per-connection is simpler; per-process would let one connection cancel another's session by id (not desired today, but federation later might).
4. **Should `Subscribe` be deprecated** in favor of bidi-with-empty-from-client? No — keeps the simpler primitive available, and stream is more efficient on the wire (no session_id round trip per frame).

---

## Plan after approval

Single PR (`C-M3a`), commits split as:
1. `frames.rs` + `CallMode::Bidi` (no behavior).
2. `LocalAbilityRegistry.register_bidi / get_bidi / list_abilities` + `AbilityDispatcher::execute_bidi`.
3. `ability_proxy` bidi arms + per-connection `BidiRegistry`.
4. `server.rs` plumbing.
5. Echo handler + e2e test.
