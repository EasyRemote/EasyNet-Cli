# Invocation Unity v1 — AXIOM §2 Mapping + U1

> Plan v10.2–v10.3. Pins the invariant "every execution entry into
> the runtime is an Invocation routed through `Kernel::invoke`".

## 1. AXIOM §2 seven-parameter mapping

```
Axon                     EasyNet-Cli (src/runtime/invocation.rs)
────                     ─────────────────────────────────────────
invoke(caller,           Invocation.caller           (URA)
        callee,           Invocation.callee           (URA)
        ability,          Invocation.ability          (AbilityUri)
        subject,          Invocation.subject          (URA)
        nonce,            Invocation.nonce_hex        (16 bytes hex)
        causal_context,   Invocation.causal_context   (enum: Null|Scalar|List|Merkle)
        args)             Invocation.args             (serde_json::Value in v1;
                                                       proto-encoded bytes in v2)
   → receipt             Receipt                     (terminal + events + prior + sig)
```

`invocation_id = sha256(canonical_bytes(inv))` is the single
identifier across IPC, Kernel, and Gateway. No layer reconstructs
or re-hashes it; downstream layers receive the same bytes the
Control layer stamped.

## 2. Signature status (v1 vs v2)

| field                         | v1 state                        | v2 state |
|-------------------------------|---------------------------------|----------|
| `Invocation.caller_signature` | `None` (field present on wire)  | filled   |
| `Receipt.callee_signature`    | `None` (field present on wire)  | filled   |

Setting either field in v1 has no runtime effect; the wire format
tolerates both for forward-compat.

## 3. In-process invariants delivered by v1

| # | Invariant                             | Mechanism                                                                 |
|---|---------------------------------------|---------------------------------------------------------------------------|
| I1| Admission exactly once                | `Kernel::invoke` nonce-dedup (5-minute local table, single-tenant)         |
| I2| Terminal monotonic                    | Receipt written once; no post-terminal `TimelineWriter::emit` is emitted   |
| I3| Receipt integrity                     | events hashed; signature optional in v1                                    |
| I4| Cancellation cooperative              | supervisor-coordinated Receipt `TerminalState::Cancelled`                  |
| I5| Event total order                     | `TimelineWriter` assigns monotonic per-invocation `sequence`               |

Across-process invariants P1–P6 are delivered by the existing
`PersistentLog` (PR-7) and are unchanged by this plan.

## 4. U1 — Kernel::invoke is the unity entry point (v10.3 C*)

Every execution entry into the runtime constructs an `Invocation`
and calls `Kernel::invoke`:

- **Client FFI** — the IPC layer seals the Client's
  `InvocationPlan` into an Invocation (adds caller URA, nonce,
  causal_context) and calls `Kernel::invoke`.
- **Schedule tick** — `execution::schedule::runner` builds an
  Invocation with `subject = schedule_id URA`, `causal_context =
  Scalar(last_receipt)` or `Null` on first fire, and calls
  `Kernel::invoke`. It does **not** call `run_mission_inproc`
  directly.
- **Loop controller** — `execution::loop_instance::runner` emits
  one Invocation per iteration (body and verify are separate
  Invocations) keyed by `subject = loop_instance URA`,
  `causal_context = Scalar(previous_iter_receipt)`. It does
  **not** drive `Session::subscribe` or the dispatch executor
  directly.
- **Permission admission** — the broker participates as an
  admission hook inside `Kernel::invoke`. A denied decision yields
  `Receipt { terminal: Failed(PermissionDenied) }`; the
  broker does not bypass Invocation construction.

This is the property that makes the audit chain referable: one id
per unit of execution, one entry point per unit of admission, one
Receipt per unit of termination.

## 5. Bypass detection

`tools/scripts/check-invocation-unity.sh` grep-enforces:

1. KernelApi / GatewayApi trait method signatures cannot take
   `args_json` / raw `Value` fragments — they must take domain
   objects (`Invocation`, `SessionId`, `PermissionRequest`, …).
2. Kernel::invoke cannot be called from inside an Execution
   sub-service (the Kernel calls sub-services, not vice versa).
3. (active, PR-INVOCATION-EXEC-UNITY) Sub-services may not bypass
   Kernel::invoke by reaching for legacy mission/session paths:
   - `execution/schedule/` cannot call `run_mission_inproc`
     (the tick runner builds an Invocation and routes through
     `Kernel::invoke`).
   - `execution/loop_instance/` cannot call `Session::subscribe`,
     `send_to_agent(...)`, or `run_mission_inproc`. The loop
     controller emits one Invocation per body / verify step.
   - `execution/permission/` cannot call `run_mission_inproc`.
     The broker is an admission hook inside `Kernel::invoke`,
     not a side-channel from elsewhere in the dispatch path.

## 6. What this plan explicitly does **not** deliver in v1

- **Scheduler policy.** v1 is tokio-backed FIFO best-effort
  dispatch. No fairness, priority, quotas, or cost-aware
  admission. `Kernel::invoke`'s admission phase leaves a hook
  position ahead of dispatch for a future scheduler layer; v1
  runs no code there.
- **Receipt-driven runtime.** `ReceiptSubscriber` is a trait
  with an empty registry. No v1 code consumes Receipts. v2
  `ReplayEngine` / `CausalScheduler` will populate the registry;
  `invocation-unity` ships the extension point alone.
- **Caller/callee signatures.** v1 wire format carries the
  optional fields; no v1 code reads or writes them.

## 7. Reviewer checklist for a new PR

- [ ] Does the PR introduce a new execution entry into the
      runtime? If yes, it must go through `Kernel::invoke`.
- [ ] Does the PR add a new KernelApi method? If yes, the
      signature must name domain objects — not `args_json`.
- [ ] Does the PR add a new Execution sub-service? If yes, it
      must be isolated per `tools/scripts/check-subservice-isolation.sh`.
- [ ] Does the PR add a new handler under `runtime/system/`? If
      yes, it must not branch on `self.node_id` /
      `target_node == self`.
