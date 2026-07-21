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
- Daemon boot progress facts are owned by the daemon event stream and
  `control.json`. The CLI may render those facts, but must not reconstruct
  omitted lifecycle fields from environment variables or old-daemon
  compatibility hints.

## Ownership

- Axon owns generic runtime semantics: descriptor binding, canonical envelope,
  admission, replay, receipts, lifecycle vectors, and language-neutral
  conformance evidence.
- EasyNet-Cli owns product/device policy: daemon lifecycle, key-service
  custody, plugins, MCP, EAL/Mission, pages, media, local resources, and route
  locality.
- Product front-door installers write only live downstream contracts. CLI
  install commands must not accept, print, or silently drop retired options
  whose semantics are not implemented by the spawned runtime surface.
- Agent registry load is a canonical read-model boundary. Loaded rows must
  already carry the current schema stamp and canonical `root_path`; production
  reads must not create agent directories, write migration backups, or infer
  roots from historical workspace layout.
- Agent lifecycle model updates are explicit state transitions.
  `agent.start` must read `model_present` as the caller's declared write
  intent and must not infer that intent from the presence of a `model` value.
- Federation owner projection publication is a write-side runtime state
  transition. `federation.advertise_abilities` must require the daemon-owned
  ability catalog sink and must not acknowledge success when the projection
  read model is unavailable.
- Federation owner projection heartbeat is a lease-refresh state transition.
  `federation.heartbeat` must require the daemon-owned ability catalog sink and
  must not report an active refreshed state from a missing projection authority.
- Federation directory and namespace route resolution are read-model authority
  queries. They must require the daemon-owned ability catalog sink; an empty
  catalog is a valid negative fact, but a missing catalog must not be projected
  as an empty ability set or stable no-route answer.
- Federation revoke is a removal state transition across presence, hosted-agent
  advertisement rows, and owner projection read-model rows. `federation.revoke`
  must require the daemon-owned ability catalog sink; a missing catalog must
  not produce `ack=true` while leaving stale route projections visible.
- Federation hosted-agent identity publication has one authority field:
  `signing_authority`. `federation.advertise_agent` must reject retired
  top-level `host_ura` ingress instead of repairing it into a hosted signing
  authority.
- FFI descriptor resolution may use the daemon-local system descriptor catalog
  only for the local runtime owner. Remote descriptor resolution must consult
  the owner route/read model through descriptor-bound runtime calls and must
  not synthesize target-owned system descriptors from static catalog shape.
- SDK packages expose canonical runtime concepts only. Product feature
  families belong to downstream providers or daemon plugins.
- Directory capability in the canonical SDK is a generic runtime projection:
  records, resolver answers, cursor state, and raw event facts. Product Hub
  directory DTOs such as agent summaries, node rows, host endpoints, and
  signing-authority variants must not be SDK public API or private SDK wire
  models.
- Invocation history point reads are canonical Axon ledger reads. The
  `invocation.history.get`, `invocation.record.get`, and
  `invocation.trace.get` key schema contains only canonical ledger keys
  (`ura`, `request_id`, `trace_id`). Pre-runtime attempt diagnostics may be
  projected into list rows when explicitly requested, but an `attempt_id` is
  not a canonical history get key and must not route into a second ledger
  authority.

## Terminology and Schema

- URA is the only active routable identity/address vocabulary.
- Transport-library `Uri`/`.uri()` usage is allowed only for HTTP/gRPC routing
  APIs, not semantic runtime identity.
- Canonical SDK public API exposes one URA type spelling per language. In Go,
  the canonical value type is `URA`; source-compatible aliases such as `Ura`
  are legacy compatibility surfaces and must not appear in source, inventory,
  or parity evidence.
- Protocol schema has one editable source; local checked-in copies must be
  mechanically derived and verified.
