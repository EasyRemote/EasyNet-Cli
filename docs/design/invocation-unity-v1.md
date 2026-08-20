# Invocation Unity

**Status:** current architecture note.
**Date:** 2026-07-11.

The retained file path is linked by architecture guards. Its contents describe
the current model: every executable request is one complete, signed Invocation
whose canonical semantics are owned by Axon.

## 1. Single execution unit

An Invocation is the indivisible seven-tuple:

1. caller URA;
2. callee URA;
3. descriptor reference, including descriptor version;
4. subject URA;
5. nonce;
6. causal context;
7. arguments.

Metadata, timeout, content metadata, authority material and signatures may bind
the tuple, but they cannot replace a tuple member. `DaemonInvocation` is the
daemon policy projection of this Axon-owned value; it is not a second canonical
format.

## 2. Construction and transport

Public callers construct the complete tuple through the generic SDK or C ABI
v5 builder. Preparation freezes the tuple and returns Axon canonical signing
material. Submission accepts either a caller-signed Invocation or the explicit
local-daemon signing path. There is no unsigned product shortcut.

The same Invocation crosses:

- `daemon.sock` ingress;
- admission and authority verification;
- route resolution;
- local runtime dispatch or cross-shard `InvocationRelay`;
- unary, server-stream or bidi execution;
- terminal receipt projection.

Adapters must preserve caller, callee, descriptor reference, subject, nonce,
causal context, arguments and admitted metadata. Reconstructing a call with a
system caller, generating a new nonce, dropping causal context, or reducing the
request to `tool + args` violates this boundary.

## 3. Stage 1 / Stage 2 separation

Stage 1 resolves the governed descriptor, authority, destination and transport
mode. Stage 2 executes the resolved Invocation. Ability handlers consume the
resolved target; they do not derive locality from node identifiers or rebuild
identity from ability names.

`control.sock` owns process lifecycle and diagnostics only. `daemon.sock` owns
canonical Invocation ingress, and admitted local calls execute directly in the
daemon's embedded Axon `LocalRuntime`.

## 4. Signing and receipts

Caller signature verification binds Axon's canonical descriptor-bound bytes.
Admission and terminal receipts bind the descriptor version, authority proof,
implementation facts, input/output hashes and causal predecessors. A receipt is
an execution fact, not a substitute response model and not an SDK history
profile.

Terminal state is monotonic. Success, failure and cancellation are mutually
exclusive. Stream and bidi handles emit at most one terminal outcome, then
close idempotently.

## 5. Lifecycle invariants

- builders transition from building to frozen exactly once;
- prepared values cannot execute directly;
- signed values transition to submitted exactly once;
- cancellation is explicit and cannot be reported as success;
- route, catalog, admission or provider failure fails closed;
- a multi-stage local registration rolls back completed stages in reverse
  order.

These invariants are enforced by `check-daemon-invocation-migration.sh`,
`check-invocation-unity.sh`, `check-dispatch-boundary.sh`, the complete-tuple
round-trip tests and the generic ABI export allowlist.

## 6. Scheduler and planner position

Scheduling policy may run between admission and dispatch, but it consumes and
returns the same complete Invocation. A planner is a caller of the governed
runtime surface: it may produce downstream Invocations, never bypass admission
with raw handler arguments or invent another execution record.

## 7. Review checklist

- Does every new execution entry accept a complete Invocation?
- Does it preserve the seven-tuple and caller signature material?
- Does routing reuse the descriptor's canonical transport mode?
- Does the handler avoid locality and identity reconstruction?
- Are terminal and cancellation transitions monotonic?
- Is any newly introduced envelope, causal-context or call-mode type actually
  a duplicate of the Axon/daemon canonical owner?
