# Formal Model of EasyNet-Cli v1 — Invocation DAG (D1) + Honest Classification (R1)

> Plan v10.5 R1. Pins the system-as-model vocabulary. Any feature PR
> that would violate one of the invariants below is rejected as
> out-of-scope for v1.

## 1. System state — five-tuple definition

At any instant the system state is

```
Σ = (N, A, I, R, E)
```

where

| symbol | name         | contents                                                                |
|--------|--------------|-------------------------------------------------------------------------|
| `N`    | Nodes        | the Axon node URAs currently visible (this device + federation peers)   |
| `A`    | Abilities    | registered callable abilities, each bound to a publisher node           |
| `I`    | Invocations  | AXIOM §2 seven-parameter tuples `(caller, callee, ability, subject, nonce, causal_context, args)` |
| `R`    | Receipts     | terminal records for each `i ∈ I` that has concluded                    |
| `E`    | Events       | per-invocation timeline frames with monotonic sequence (AXIOM §6.1 I5)  |

`invocation_id(i) = sha256(canonical_bytes(i))` is the single
system-wide key shared by the IPC `request_id`, the boot KernelApi
in-flight table, and (when the dispatch is remote) the Axon
`send_a2a_task` identifier.

## 2. Invocation DAG

Let `G = (V, E_causal)` where

- `V = I`
- `(i, j) ∈ E_causal` iff `j.causal_context` references `i`'s
  receipt (under any of the four `CausalContext` shapes
  `Null | Scalar | List | Merkle` from AXIOM §2.x).

### D1 (acyclic)

`G` is a DAG. Because `j.causal_context` cites a prior receipt and
prior receipts are by construction produced before `j` is admitted,
the directed relation is consistent with wall-clock time ordering
on a single node and with AXIOM §6.1 I1 admission-exactly-once
across peers.

### D2 (single callee signature per receipt)

Each `r ∈ R` carries at most one `callee_signature`. In v1 the
field is always `None` (v2 fills it in under AXIOM §6.3); the
uniqueness claim is structural regardless of whether the field is
populated.

## 3. Four invariant groups

Every PR-DAEMON/-SYS/-ATTACH/-PERM/-DISCUSS/-SCHED/-LOOP/-INVOCATION-EXEC-UNITY
change is checked against the table below. Rows are grouped by
class so a reviewer can answer "did this PR drift v1 toward what
v1 is *not*" quickly.

### 3.1 Structural / operational — v1 delivers

| #   | Invariant                              | v1 status                                           | Enforcement / evidence                                         |
|-----|----------------------------------------|-----------------------------------------------------|----------------------------------------------------------------|
| I1  | Admission exactly once                 | ✅                                                  | `daemon::boot::kernel::Kernel::invoke` nonce-dedup (PR-INVOCATION-EXEC-UNITY) |
| I2  | Terminal monotonic                     | ✅                                                  | Receipt written once; no post-terminal event emit              |
| I3  | Receipt integrity (events hashed)      | ⚠️ partial (hash present; callee_signature empty)   | `daemon::invocation::receipts::runtime_record::Receipt` + events field |
| I4  | Cancellation cooperative               | ✅                                                  | supervisor-coordinated cleanup path                            |
| I5  | Event total order                      | ✅                                                  | monotonic `sequence` per invocation in `timeline.rs`           |
| P1  | Append-only log                        | ✅                                                  | PR-7 `PersistentLog`                                           |
| P2  | Durability before notify               | ✅                                                  | `TimelineWriter::emit` (fsync before broadcast)                |
| P3  | Offset-read contract                   | ✅                                                  | `system.session.attach(run_id, since_seq)`                     |
| P4  | Terminal idempotent                    | ✅                                                  | Receipt byte-stable on re-read                                 |
| P5  | Explicit eviction                      | ⚠️ partial (no automatic eviction)                  | manual `runs/` cleanup                                         |
| P6  | Crash consistent                       | ✅                                                  | `PersistentLog`                                                |
| D1  | DAG acyclic                            | ✅                                                  | causal_context wall-clock ordering                             |
| D2  | Single callee signature per receipt    | ⚠️ partial (signature empty in v1)                  | `Kernel` state machine + Receipt field optionality             |
| U1  | Invocation unity (one exec entry)      | ✅ (after PR-INVOCATION-EXEC-UNITY)                 | `daemon::boot::kernel::Kernel::invoke` + `tools/scripts/check-invocation-unity.sh` |

### 3.2 Non-repudiation — v2 target

| #  | Invariant                                   | v1 status   | v2 path                                        |
|----|---------------------------------------------|-------------|------------------------------------------------|
| C1 | Caller signs canonical bytes                | ❌          | populated when signed-invocation lands          |
| C2 | Callee signs receipt                        | ❌          | populated when signed-invocation lands          |

### 3.3 Semantic — v1 deliberately does **not** deliver (R1)

| #  | Invariant                                                                              | v1 status          | Note                                              |
|----|----------------------------------------------------------------------------------------|--------------------|---------------------------------------------------|
| S1 | Causal determinism: DAG history + ability def ⇒ response is computable                 | 🔴 not delivered    | runtime does not consume history as input        |
| S2 | Receipt utility: a new Receipt changes observable runtime state                        | 🔴 not delivered    | Receipt is a durable record, not a driver        |
| S3 | Replay soundness: receipts suffice to reconstruct the state they represent             | 🔴 not delivered    | no replay engine                                 |
| S4 | Causal consistency: invocations citing the same prior receipt share runtime context    | 🔴 not delivered    | runtime ignores prior receipts                   |

## 4. Honest Classification

**Chinese (plan v10.5 R1 core declaration):**

> v1 兑现结构不变式与工程不变式（D1, D2, I1–I5, P1–P6, U1）。v1
> 不兑现语义不变式（S1–S4）。Receipt 是 durable record，不是 runtime
> 决策的 semantic input。因此 v1 是 record system，不是 computation
> system。这是自觉选择，不是疏漏。
>
> 实现层面上这表现为"开环"（receipt 写盘但不反馈），但真正的本体判断
> 不是"开环 vs 闭环"，而是"runtime 是否把历史当作可计算输入" — v1
> 不把历史当作可计算输入。

**English (suitable for papers and external design docs):**

> v1 satisfies structural and operational invariants only (D1, D2,
> I1–I5, P1–P6, U1). It does not satisfy semantic invariants (S1
> causal determinism, S2 receipt utility, S3 replay soundness, S4
> causal consistency). Receipts are durable records, not semantic
> inputs to runtime decisions. Therefore v1 is a **record system**,
> not a computation system. This is a deliberate framing, not an
> omission.
>
> The implementation-level descriptor "open-loop" is secondary; the
> ontological distinction is whether the runtime consumes history as
> a computational input. v1 does not.

## 5. Framing boundary — what v1 may / may not be sold as

**May be framed as:**
- "invocation-first, audit-grade, structurally well-formed runtime /
  record substrate"
- "single-node agent runtime with Invocation DAG persistence"
- "Paseo-parity product + AXIOM-aligned invocation unity"

**May not be framed as:**
- "receipt-driven, causally meaningful computation system" ❌
- "new computation model" ❌ — the novelty is about protocol /
  record / audit substrate, not about receipts participating in
  computation
- "complete AXIOM implementation" ❌ — C1/C2/S1–S4 are explicitly
  not delivered

Any paper or whitepaper using v1 as ground truth must land its
novelty claim inside the "protocol/record/audit substrate" category.
This is not a suggestion; it is the ceiling on the framing v1
supports.

## 6. Upgrade paths to v2

Two orthogonal paths, each independently schedulable:

- **S-path (semantic upgrade).** Register one or more
  `ReceiptSubscriber` implementations that turn Receipts into
  runtime inputs. Minimum: satisfy S2 (observable state change).
  Full: satisfy S1/S3/S4.
- **C-path (non-repudiation upgrade).** Populate
  `Invocation.caller_signature` and `Receipt.callee_signature`
  with Ed25519 signatures per AXIOM §6.3. Satisfies C1/C2; closes
  the audit chain.

v1 code preserves the extension points (the `ReceiptSubscriber`
trait and the optional signature fields) so either path lands
additively. This plan does **not** schedule either path; that
belongs to a successor plan.
