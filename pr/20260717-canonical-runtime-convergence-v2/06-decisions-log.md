# Canonical Runtime Convergence V2 - Decisions Log

## 2026-07-17

- Descriptor governed schema inputs are modeled as one projection object
  because they define one hash/proof fact boundary. A positional helper made
  the descriptor hash inputs easier to reorder or partially duplicate.
- The old scatter-argument helper was removed instead of retained as a
  compatibility layer because no canonical domain should expose two ways to
  assemble the same descriptor proof material.
- This slice intentionally does not claim RF-3 closure. It removes one local
  descriptor assembly fork, while public plain admission/signature exports and
  cross-language proof cutover remain open.
- Mission terminal state belongs in EasyNet-Cli because Mission/EAL is daemon
  product orchestration. The refactor keeps that lifecycle explicit as daemon
  state while avoiding any new Axon Mission ontology or alternate invocation
  proof path.
- `Kernel::default()` delegates to `Kernel::new()` because the allow-all local
  service graph is the default object lifecycle. Subscriber-broker construction
  remains a named daemon boot policy path.
- Stream/bidi event enums use boxed large payloads at channel and classifier
  boundaries. This keeps the admission/chunk/terminal state machine unchanged
  while bounding queue element size.
- Reverse session escalation boxes canonical `InvokeResponse` replies because
  they carry proof material and should not define the fixed size of every
  control reply slot. Ready hooks are named as session outbox lifecycle types.
- Dispatch result projection must enumerate canonical carrier fields at the
  bridge/session boundary. No-op default tails are removed because they obscure
  whether receipt and failure fields are intentionally projected.
- Resolver plans use `InvocationPlanIngress` rather than `Option<subject>`
  because public ingress and daemon-system calls have different authority
  sources. Only the daemon-system variant may select root causal context and
  descriptor-derived subject policy.
- `InvocationTarget` construction is owned by the target value object for
  common local dispatch states. Edge adapters should call named constructors
  instead of repeating local scope, root causal context, and empty metadata
  literals.
- Plugin host tests use the same named target constructors as production
  adapters so test fixtures do not preserve the obsolete local target assembly
  idiom.
- Resource and governance adapters select ability, payload, and subject; the
  routing target value object owns local scope, system causal policy, and empty
  metadata construction.
- Media subject-boundary fixtures should use routing target constructors so
  missing-subject, wrong-type, corrupt-table, and subject-in-args tests do not
  preserve obsolete local target assembly.
- Camera snapshot, subscribe, and recording fixtures follow the same routing
  target constructor boundary as mic and screen because all media handlers
  share the same envelope-subject policy.
- Subject derivation for daemon-system LocalRuntime calls belongs to
  `InvocationTarget`, not `local_runtime_invoker`, because target resolution
  owns the public-ingress versus daemon-system tuple source. The LocalRuntime
  adapter should build Axon requests from resolved tuple facts, not define a
  second fallback subject policy.
- Mission production child dispatch remains descriptor-bound through the
  admitted parent `AbilityContext`; the test catalog gateway is only a unit
  test port. Even there, local target construction should use the routing
  target constructor so Mission/EAL fixtures do not preserve obsolete tuple
  assembly examples.
- Ability dispatch registry tests are part of the canonical daemon routing
  surface. They should construct local and remote daemon-system targets through
  `InvocationTarget` constructors so the test suite does not teach future
  adapters to restate tuple policy by hand.
- LocalRuntime invoker tests verify Axon request lowering, not routing target
  construction. Their fixtures should call `InvocationTarget` constructors so
  the adapter module does not preserve a parallel target assembly example.
- Broad built-in smoke tests and catalog assembly tests should not expose
  handwritten local target literals because those helpers become examples for
  future adapter tests. The target value object remains the owner of
  daemon-system tuple policy.
- CLI command fixtures that dispatch into the daemon ability catalog should
  also use routing target constructors. Envelope-aware handler tests may keep
  explicit `EnvelopeContext` fixtures because they validate handler context,
  not target construction.
- Protobuf `EnvelopeOpen.target` construction belongs in the invocation wire
  facade. It is a transport selector projection, not the canonical invocation
  tuple, so concentrating it in `wire_invocation_target` removes duplicated
  proto assembly without changing the signed envelope's ownership of caller,
  callee, subject, nonce, causal context, and descriptor-bound proof material.
- External integration tests may still build raw protobuf fixtures when they
  model an outside client. That does not justify keeping internal daemon or SDK
  transport adapters hand-building the same target selector literal.
- The SDK capability matrix must not count process-local generated subject auth
  as canonical runtime evidence. Such helpers are RF-5 defects until migrated
  to explicit signer-handle or daemon KeyService authority, so the conformance
  model must quarantine them instead of classifying them under normal
  capability ownership.
- Public-surface quarantine alone is not implementation cutover. It prevents
  false readiness claims and new canonical capability evidence for fallback
  signers; after the Rust public fallback root removal, RF-5 still requires
  cross-language signer-handle parity and daemon KeyService authority cutover.
- The Rust Axon SDK runtime-admin public surface must not mint process-local
  signing secrets. `GeneratedSubjectAuth` and the generated private
  agent/hub/subject auth helpers are removed instead of retained behind a
  compatibility layer because they define the wrong authority model. Subject
  identifier helpers remain only as pure string construction.
- Fallback signer detection is a class-level conformance rule, not a single
  symbol rule. The V2 gate now rejects generated subject auth and generated
  private agent/hub auth if any language reintroduces them into canonical
  symbols or members.
- Plain invocation bytes and plain admission helpers are not compatibility
  surfaces inside the canonical SDK domain. They may exist only as
  crate-internal test fixtures for historical vector stability; any public
  Rust/Python export must use descriptor-bound proof.
- Public-surface quarantine is too weak for RF-3 after the Rust/Python public
  removal. The V2 gate now fails if plain proof helpers appear anywhere in the
  public manifest, including `non_canonical`, because the clean target is
  absence rather than documented legacy.
- Runtime-admin resolver tests should verify bootstrapped keys through
  `DescriptorBoundEnvelope` and descriptor-bound signature verification. A
  resolver test that signs plain invocation bytes preserves the wrong proof
  boundary as an example for future code.
- Python submodule functions are SDK surface unless marked private. Removing a
  name from package-root `__all__` is insufficient for RF-3 when
  `easynet_axon.invocation.axiom.sign_invocation` remains importable as a
  normal function.
- Historical plain axiom vectors may keep private fixture helpers while the
  vector migration is still open. The naming must make the boundary explicit:
  private fixtures prove legacy byte stability; descriptor-bound helpers are
  the public proof path.
- Manifest inventory is necessary but not sufficient for Python proof-boundary
  convergence. The V2 gate must also scan Axon source for public plain helper
  definitions and re-exports.
- Java receipt constructors rejecting omitted proof facts is not enough if
  `LocalRuntime` still supplies `ReceiptProofFacts.empty()`. RF-6 must be
  enforced at the production binding boundary where receipts are actually
  emitted.
- Signed Java LocalRuntime receipt facts are descriptor-bound admission facts:
  subject, descriptor version, authority proof, input hash, parent receipts,
  and runtime env are derived from the admitted envelope rather than attached
  during serialization.
- Plain Java `invokeAsync` is a system-local runtime path, not an external
  descriptor-bound caller. Its receipts use a separate
  `system-local.invoke.v1` proof identity so internal SystemAgent calls remain
  auditable without creating a compatibility fallback or mislabeling their
  authority source.
- Receipt output hash belongs to the emitted event, not the admission binding.
  `AxiomBinding.withPayloadDigest` therefore replaces immutable proof facts
  with an event-specific output hash when `InvocationCore` emits each receipt.
- The V2 RF-6 gate must scan production runtime code, not only receipt
  constructors and bundle parsing. A constructor can be strict while a runtime
  caller still passes empty facts.
- Python LocalRuntime follows the same RF-6 ownership as Java: receipt proof
  facts are binding-domain facts created before invocation launch, while
  per-event output hash is refreshed at receipt emission.
- Python `invoke_async` is an internal SystemAgent path. It needs complete
  proof facts, but those facts must use a system-local proof identity rather
  than pretending the call was an externally signed descriptor-bound request.
- Python source scans must include `local_runtime.py`; checking only
  `axiom.py` and `audit.py` can miss a runtime that passes strict constructors
  while still emitting receipts with an empty default proof object.
- Go LocalRuntime follows the same RF-6 ownership as Java and Python: receipt
  proof facts are constructed at the admitted binding boundary, while
  per-event output hashes are refreshed when `InvocationCore` emits receipts.
- Go `InvokeAsync` is a system-local runtime path. It gets complete proof
  facts under `system-local.invoke.v1` rather than reusing empty facts or
  pretending to be an externally signed descriptor-bound call.
- Go and Rust must share the same descriptor-bound ability identity. The Go
  parser now requires `ability_ura@version#descriptor_hash!admission_action`
  so a Go-signed bundle can be verified by the Rust verifier without a plain
  proof fallback.
- The V2 RF-6 gate must reject `EmptyReceiptProofFacts()` in Go production
  LocalRuntime code; test fixtures can remain explicit until the broader
  constructor/test cleanup slice closes RF-6 globally.
- Go runtime-level lifecycle controls must not bypass handle controls. The
  runtime `Cancel` and `SendMessage` APIs resolve the current generation token
  and enter the same control state machine used by `InvocationHandle`.
- `CoreOf` is an inspection surface for audit/lifecycle vectors, not a second
  mutation API. Mutating lifecycle operations remain owned by LocalRuntime and
  InvocationHandle control methods.
- Node LocalRuntime follows the same RF-6 ownership as Java, Python, and Go:
  receipt proof facts are constructed at the admitted binding boundary, while
  per-event output hashes are refreshed when `InvocationCore` emits receipts.
- Node `invokeAsync` is a system-local runtime path. It gets complete proof
  facts under `system-local.invoke.v1` rather than reusing empty facts or
  pretending to be an externally signed descriptor-bound call.
- The V2 RF-6 gate must reject `EMPTY_RECEIPT_PROOF_FACTS` in Node production
  LocalRuntime code; the sentinel may remain only as an explicit fixture value
  for tests and non-production construction audits.
- Rust `local-fast-probes` is not a canonical SDK capability. It exposes a
  process-local signer fallback through a public Cargo feature, so it must be
  removed rather than documented as a compatibility surface.
- Test-only signer convenience belongs in test fixtures, not in SDK public
  architecture. Rust integration tests now own explicit descriptor-bound test
  providers in `descriptor_bound_support`, while crate-local helpers remain
  `cfg(test)` only.
- Examples should demonstrate the provider-backed runtime model. The
  `receipt_closure` example now constructs an explicit receipt signing
  authority provider instead of relying on `new_local_fast`.
- The V2 RF-5 gate must reject both the public Rust feature and external
  consumption of local fallback helpers. Source-level absence is required
  because API manifest inventory alone cannot see Cargo feature gates or
  examples/tests that preserve the wrong authority model.
- EasyNet-Cli must not keep a downstream `local-fast-probes` feature after the
  Axon SDK removes it. A compatibility feature would preserve the wrong
  architecture even if production binaries did not enable it.
- The manual `real-user-smoke` binary may own a local explicit receipt
  provider because it is a downstream maintainer probe, not SDK architecture.
  The provider stays file-local so it cannot become a reusable fallback
  surface.
- Pages integration tests should not force an Agent owner into
  `ProductionReceiptAuthorityConfig`; production self-signed owners are Hub or
  Device roots. The Pages test therefore owns a bounded Pages-agent test
  provider instead of misusing production owner configuration.
- EasyNet-Cli lib tests cannot import Axon's `cfg(test)` helper re-exports
  because dependency crates are not compiled with the downstream crate's test
  cfg. CLI test runtime, Mission child signing, and local-daemon gRPC receipt
  fixtures now define their own explicit providers at the fixture boundary.
- `AxonClient::generate_subject_auth` is removed because SDK-generated
  subject secrets are a process-local signer fallback, not a canonical runtime
  authority model. Keeping it as a wrapper would preserve the wrong ownership
  even if callers were encouraged not to use it.
- `EasyNetUserAuth` remains only as an explicit host-supplied DTO while RF-5
  signer-handle convergence is still open. The clean target is signer handles
  or daemon KeyService authority, not an SDK helper that mints private keys.
- Runtime client SDK tests use fixed `host_auth_fixture` material so the test
  authority is local to the test and visibly injected, rather than generated
  by public SDK API.
- The V2 RF-5 source gate scans runtime client SDK source in addition to the
  canonical SDK manifest because process-local signer fallback helpers can
  live outside the language package inventory.
- Go plain proof helpers are package-private fixtures, not SDK proof APIs.
  Keeping package-local historical vector coverage is acceptable only because
  exported descriptor-bound helpers are the public proof path.
- The SDK API mapping must document descriptor-bound proof names. Leaving
  plain helper names in the mapping would preserve the wrong public contract
  even after source exports were removed.
- The V2 RF-3 gate now checks Go source and API mapping for the capitalized
  plain helper group because Go export semantics are name-based and cannot be
  inferred from manifest inventory alone.
- Node root and invocation entry points must not export camelCase plain proof
  helpers. Generated declarations are part of the public boundary, so removing
  TypeScript exports without rebuilding declarations is incomplete.
- Node historical plain vector coverage is retained only through explicitly
  named `legacyPlain*` fixture helpers. The name makes the non-canonical role
  visible and prevents tests from teaching the old public API shape.
- Node cross-language bundle production must sign descriptor-bound canonical
  bytes and use descriptor-ref ability names. Rust `easynet-verify` rejecting
  plain-signed Node bundles is correct RF-3 behavior, not a compatibility
  problem to relax.
- The V2 RF-3 gate now scans Node source, generated JS, and generated
  declarations for the retired plain helper names because Node package exports
  alone are too indirect to prove the public proof boundary.
- Java public static methods are SDK public API. Plain proof/admission helpers
  may remain only as package-private `legacyPlain*` fixtures for same-package
  historical vector tests.
- Java cross-language bundle production must use descriptor-ref ability names
  and sign descriptor-bound canonical bytes. A Java bundle accepted only by a
  plain verifier would preserve RF-3's second proof model.
- The V2 RF-3 gate now scans Java production invocation classes for public
  static retired helper names because Java package-private fixture methods are
  valid test internals but public static methods are facade API.
- Swift top-level `public func` declarations are SDK public API. Plain
  proof/admission helpers may remain only as internal `legacyPlain*` fixtures
  for same-module historical vector tests.
- Swift cross-language bundle production must use descriptor-ref ability names
  and sign descriptor-bound canonical bytes. A Swift bundle accepted only by a
  plain verifier would preserve RF-3's second proof model.
- Swift public examples and README snippets are part of the effective SDK
  surface because users copy them as integration contracts. The V2 RF-3 gate
  therefore scans Swift production invocation source, README, and examples for
  retired plain helper usage.
- Go package-private names are still architecture signals inside the canonical
  SDK package. Keeping `canonicalInvocationBytes`, `signInvocation`,
  `verifyInvocationSignature`, `verifySignature`, or `runAdmission` as normal
  production helper names preserves RF-3's second proof model even when those
  symbols are not exported.
- Go historical plain vector coverage may remain only through explicit
  `legacyPlain*` fixture names. The V2 RF-3 gate now rejects retired Go plain
  helper names in non-test invocation source while allowing tests to exercise
  the renamed legacy fixtures.
- Rust `cfg(test)` helper names still shape the canonical package vocabulary.
  Retired plain proof/admission helpers must therefore use explicit
  `legacy_plain*` names rather than ordinary proof/admission names, even when
  the functions are not public exports.
- The Rust signature-bytes verifier remains neutral because descriptor-bound
  verification and legacy plain fixture verification both need the same
  Ed25519/key-resolver mechanics. The legacy boundary is the bytes selected
  for verification, not the generic signature checker.
- The V2 RF-3 gate now rejects retired Rust plain helper names anywhere under
  `sdk/rust/src/invocation`; this is stricter than public-surface inventory
  because package-internal names are architecture examples for future code.
- Python private helper names are also architecture signals inside the
  canonical SDK package. Retired plain proof/admission helpers must therefore
  use explicit `_legacy_plain*` names rather than ordinary proof/admission
  names, even when they are not exported from the package.
- Python cross-language bundle production must sign descriptor-bound canonical
  bytes and use descriptor-ref ability names. A Python bundle accepted only by
  a plain verifier would preserve RF-3's second proof model.
- The V2 RF-3 gate now scans all Python SDK source for retired private plain
  helper names because public inventory alone cannot prove package-internal
  proof vocabulary convergence.
- Node `legacyPlain*` functions are not acceptable in production invocation
  source even when the package root does not re-export them. Source-level
  module exports are still SDK contract surface for package consumers and
  future internal code.
- Historical plain vector coverage may remain in Node only behind an explicit
  test/vector fixture boundary. The canonical Node invocation modules now host
  descriptor-bound proof and admission only.
- Go `legacyPlain*` functions are not acceptable in production invocation
  source either. Package-private production helpers still shape the canonical
  runtime model and would preserve RF-3's second proof/admission
  implementation.
- Historical plain vector coverage in Go may remain only in `_test.go`
  fixtures. The Go runtime package now hosts descriptor-bound signing,
  verification, and admission as its only production proof boundary.
- The V2 RF-3 gate now rejects Go legacy plain proof/admission names in
  non-test invocation source because public API inventory cannot detect
  package-private production implementation residue.
- Protocol-pack conformance vectors are active runtime contracts, not
  historical prose. A URA grammar vector must therefore use URA vocabulary in
  its file name, description, and JSON field names.
- The RF-9 gate now rejects URI-named Axon protocol-pack URA vectors. Transport
  library `Uri` types remain a separate implementation concern; active
  routable identity/address artifacts use URA naming.
- `AXIOM.tex` and RFC-001 are active invocation contracts, not historical
  migration notes. Their identity composite examples must therefore use
  `ura`, `caller.ura`, and URA profile terminology to match the canonical
  proto surface.
- Changing active normative document vocabulary from URI to URA does not
  change field numbers or canonical byte ordering; it removes a documentation
  fork that contradicted the current `AgentIdentity.ura` schema.
- RFC-002 keyring and KeyResolver examples are also active authority-boundary
  contracts. Peer-table projections and resolver pseudocode must therefore use
  `peer_ura` and `find_peer_by_ura` rather than preserving URI vocabulary.
- React `useAbilityTools` is a product tool-provider bridge, not a canonical
  runtime primitive. Removing it from source, root exports, README, and skill
  guidance is required; keeping a hook shim would preserve RF-1 inside the SDK.
- Generated declarations are part of the public SDK surface. The React type
  build now clears `dist/types` before generation so stale `tool_adapter`
  declarations cannot survive after the source and export are deleted.
- RF-1 product-surface gates must cover every language facade. React was
  missing from the earlier product-boundary inventory, so the V2 gate now
  rejects tracked React `tool_adapter` artifacts and public React
  `useAbilityTools` documentation.
- Active proto comments are schema-source vocabulary, not incidental prose.
  The `federation.list_user_devices` comment must say device URAs because the
  field is a URA realm and downstream generated SDKs inherit that contract.
- Proto mirrors must be synchronized from `core/proto/axon/v1` rather than
  manually edited. The RF-9 proto terminology fix therefore updates the
  canonical source first and derives the runtime client-sdk and Rust SDK
  mirrors through the existing syncer.
- The V2 RF-9 gate now scans Axon active proto roots for URI identity
  vocabulary because document/vector gates alone cannot prove schema-source
  terminology convergence.
- SDK validation errors are public contract text. Error messages that say
  `EasyNet URA` make the canonical runtime facade product-specific even when
  the underlying value is a generic URA.
- Product-neutral SDK wording uses `canonical URA` for complete subject values
  and `URA syntax` for legacy private principal rejection. The scheme literal
  remains unchanged; this slice changes the contract vocabulary, not wire
  semantics.
- Active source identifiers follow the URA-only naming rule. Swift
  `SYSTEM_URI` is therefore renamed to `SYSTEM_URA` rather than left as a
  harmless-looking local constant.
- The V2 gate now rejects product-specific URA error vocabulary in active SDK
  source because manifest inventory cannot see public error-message contracts.
- Receipt authority anchor tests are canonical runtime conformance tests, not
  language-local fixtures. The anchor input must therefore carry the same
  complete `ReceiptProofFacts` values across Rust, Java, Python, Node, and
  Swift.
- `runtime_env` is signed receipt material. Anchor fixtures must use one
  language-neutral value (`axon-receipt-anchor-v2`) rather than embedding
  `java-test`, `python-test`, `swift-test`, or similar language names.
- Empty proof facts remain a transitional defect under RF-6. Anchor tests now
  pin complete subject-ref, descriptor, schema, implementation, authority,
  input, and output proof facts instead of preserving the old empty-proof
  receipt model.
- A missing receipt authority is not equivalent to self authority. Python's
  anchor test now asserts `receipt_authority_binding_required`, matching the
  RF-6 direction that receipt construction must reject omitted authority facts.
- Python fluent receipt construction is a public receipt producer, not a test
  fixture. It must therefore require caller-supplied `ReceiptProofFacts` at
  `.call(payload, proof_facts=...)` instead of synthesizing
  `ReceiptProofFacts()` internally.
- The fluent API requires proof facts at `call` time because `input_hash` is a
  signed receipt fact derived from the payload. `ability(...)` cannot own that
  fact without guessing future payload bytes.
- Python `prove_authority()` must read the explicit authority binding already
  stored on `AxiomBinding`; constructing a dummy `ReceiptBody` preserved an
  obsolete receipt path and failed once receipt proof facts became mandatory.
