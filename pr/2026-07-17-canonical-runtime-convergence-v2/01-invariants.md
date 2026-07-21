# Invariants

## Invocation Integrity

- Every public SDK, FFI, daemon, and backend-facing invocation boundary exposes
  the full tuple: caller, callee, ability, subject, nonce, causal_context, and
  args.
- `subject` and `causal_context` are explicit values or explicit named
  derivation policies. Silent callee/descriptor/empty-causal substitution is a
  defect.
- Internal daemon calls use a named system issuer, daemon key custody, and the
  same descriptor-bound admission path as external calls.

## Proof and Receipt

- Descriptor-bound invocation is the only canonical admission input.
- Plain canonical-byte signing, plain signature verification, and plain
  admission are not public SDK/runtime surfaces.
- Receipt construction requires caller-supplied authority and proof facts.
  Encoders and parsers must not synthesize empty authority or proof facts.

## Lifecycle

- Invoke, stream, bidi, and child dispatch paths have one deterministic
  terminal state: success, rejection, failure, cancellation, or deadline.
- Cancellation and deadline handling are idempotent and observable through a
  terminal receipt or terminal event.
- Queueing, pending dispatch maps, and stream buffers have hard capacity
  bounds.

## Ownership

- Axon owns generic runtime semantics: descriptor binding, canonical envelope,
  admission, replay, receipts, lifecycle vectors, and language-neutral
  conformance evidence.
- EasyNet-Cli owns product/device policy: daemon lifecycle, key-service
  custody, plugins, MCP, EAL/Mission, pages, media, local resources, and route
  locality.
- SDK packages expose canonical runtime concepts only. Product feature
  families belong to downstream providers or daemon plugins.
- Directory capability in the canonical SDK is a generic runtime projection:
  records, resolver answers, cursor state, and raw event facts. Product Hub
  directory DTOs such as agent summaries, node rows, host endpoints, and
  signing-authority variants must not be SDK public API or private SDK wire
  models.

## Terminology and Schema

- URA is the only active routable identity/address vocabulary.
- Transport-library `Uri`/`.uri()` usage is allowed only for HTTP/gRPC routing
  APIs, not semantic runtime identity.
- Protocol schema has one editable source; local checked-in copies must be
  mechanically derived and verified.
