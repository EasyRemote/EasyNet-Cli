# Invariants

- SDK internals use generic runtime names and generic lifecycle concepts.
- Public invocation boundaries preserve the full seven-field tuple.
- Product-specific daemon, EasyNet, EasyRemote, device, Hub, plugin, or receipt
  ownership does not become part of canonical SDK concepts.
- Legacy input shapes are either rejected deterministically or translated at a
  versioned edge into the canonical descriptor-bound request.
- Agent registry key projections are authoritative inputs to admission,
  Mission target conflict detection, and skill ownership discovery. A malformed
  registry key is corrupt aggregate state and must fail closed; product paths
  must not silently drop the bad row and continue with a partial projection.
- Owner projection cursors are durable publication lifecycle facts. Each cursor
  must bind a canonical owner URA to a canonical publisher host state: Agent
  owners are hosted by a Device in the same realm, Device owners are hosted by
  themselves, and Authority owners are hosted by themselves.
- Builtin plugin provider projection must bind three identity facts before a
  package binding exists: provider package id, manifest package id, and manifest
  entrypoint. A provider whose compiled entrypoint does not match its manifest
  is not a partially loadable plugin.
- Desktop companion daemon lifecycle reconciliation is an audit boundary. It may
  continue reconciling independent packages after one package fails, but it must
  emit an explicit failure fact for every malformed package plan or unreadable
  desired-state store. Corrupt companion state is not equivalent to "disabled",
  and an unplannable stop-on-runtime-stop companion is not equivalent to "no
  companion to stop".
- Runtime-host lifecycle detach is a required provider capability in every SDK
  language. A function adapter with no detach provider is an invalid transport,
  not a successful no-op. Idempotent local detach state belongs on the handle
  after a real provider detach succeeds; missing lifecycle authority must fail
  closed.
- Remote session handler error frames are schema-bearing terminal failure inputs.
  They must carry explicit non-empty `code` and `message` facts before the daemon
  projects them into `SessionFailure`. Missing handler error facts are dispatch
  protocol violations, not product failures with synthesized default codes or
  messages.
- SDK receipt history reads are provider-backed governance reads in every
  language. Product code must not invoke `invocation.history.list` through the
  public descriptor-bound action ingress, and SDKs must expose a runtime receipt
  provider that owns descriptor-provider selection, tuple construction, and
  authority/subject preflight.
- Runtime response state projection is a protocol boundary. A wire-level
  `InvokeResponse.state` that cannot be decoded into the canonical invocation
  lifecycle must become an explicit `PROTOCOL_MISMATCH` terminal attempt
  failure; it must never be recorded as `unknown` or left as
  `runtime_started`.
- Presence has two explicit slot states: negotiated dispatch session and
  resolve-only directory visibility. A resolve-only row must never expose a
  dispatch sender or carrier contract, and a dispatch session must carry a
  validated session contract with a 16-byte claimant fingerprint.
- Native runtime session signing has one caller-authority model. Unsigned FFI
  invocations may be signed only by the session runtime owner or by the
  Ready-proven paired User caller; both paths load signers through the canonical
  runtime caller signer resolver. FFI must not call runtime-owner-only signer
  loaders for User URAs or treat arbitrary keyring inventory as session
  authority.
- Local runtime stream projection has one chunk-authority object. Non-terminal
  frames project to explicit `Running` chunks; successful terminal frames must
  be backed by canonical `Completed` finalization with no failure; error
  terminal frames must bind the finalized terminal receipt and a concrete
  failure fact. The dispatcher loop may await lifecycle events, but it must not
  reassemble receipt/state/error wire chunks procedurally in multiple branches.
- Driver invocation trace metadata is product observability, not protocol
  truth, but every URA field it applies to a tool-call record must still be a
  canonical runtime URA. Legacy `/invocation/...` trace addresses, malformed
  URAs, and all-zero principal placeholders must be rejected at the trace parser
  boundary instead of being merged into product UI state.
