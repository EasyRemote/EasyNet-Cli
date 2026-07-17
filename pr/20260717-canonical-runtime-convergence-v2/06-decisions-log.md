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
- Java `ReceiptProofFacts.empty()` is an obsolete public receipt-construction
  helper. Keeping it after LocalRuntime and fluent receipt paths moved to
  explicit proof facts would preserve the wrong proof model as a reusable SDK
  API.
- Java examples and tests must construct descriptor/runtime proof facts where
  they construct signed receipt bodies. An example that calls an empty helper
  is effective SDK guidance, not harmless test convenience.
- Causal parent receipts are proof facts, not display-only trace metadata. The
  Java receipt-closure and verb fixtures now carry scalar/list causal parents
  into `ReceiptProofFacts.parentReceipts` instead of signing receipts with an
  empty proof tail.
- Go `EmptyReceiptProofFacts()` is the same RF-6 defect as Java's removed
  empty helper. Even if production LocalRuntime no longer calls it, keeping it
  public in the invocation package preserves a reusable no-proof receipt
  construction path.
- Go receipt tests should own explicit fixture proof facts rather than call a
  production empty helper. The fixture binds subject, descriptor, runtime,
  authority, input/output hashes, and causal parents so tests exercise the
  canonical receipt model.
- Go authority anchors are cross-language conformance tests. They must use the
  shared `axon-receipt-anchor-v2` strict proof-facts fixture rather than the
  local receipt-verbs fixture, so Go remains aligned with Rust, Java, Python,
  Node, and Swift anchor pins.
- The V2 RF-6 gate now rejects `EmptyReceiptProofFacts()` anywhere under the
  Go invocation package because a helper in tests, examples, or production
  source would teach the obsolete empty-proof model.
- Swift `ReceiptProofFacts.empty` is the same RF-6 defect as the removed Java
  and Go empty helpers. Removing only production calls would leave an SDK API
  that still teaches no-proof receipt construction, so the helper and unchecked
  receipt-facts initializer are deleted.
- Swift `ReceiptProofFacts` construction must require all receipt proof
  fields explicitly. Defaulted hashes, runtime env, authority proof, and
  parent receipts are a fallback path, not a canonical runtime abstraction.
- Swift LocalRuntime owns local receipt proof-fact construction at the binding
  boundary. Descriptor-bound and system-local receipts use separate runtime
  env/admission-hook values, while both carry subject ref, descriptor identity,
  authority proof, input hash, and causal parent receipts through one shared
  builder.
- Swift receipt output hashes are per-emitted-receipt facts. Updating only the
  `payloadDigest` on `AxiomBinding` would leave stale proof facts, so
  `withPayloadDigest` also refreshes `ReceiptProofFacts.outputHash`.
- Swift examples and tests are part of the effective SDK contract. They must
  construct explicit proof facts at the receipt signing boundary instead of
  using empty helpers or fixture-level self-authority fallback.
- The V2 RF-6 gate now scans Swift invocation source, examples, and tests for
  `ReceiptProofFacts.empty`, `proofFacts: .empty`, empty
  `try ReceiptProofFacts()` construction, and authority fallback shapes because
  public API inventory cannot detect these semantic receipt-model regressions.
- Node `EMPTY_RECEIPT_PROOF_FACTS` is a public no-proof receipt construction
  value, not a harmless fixture. Keeping it after Node LocalRuntime moved to
  explicit proof facts would preserve RF-6 as an SDK integration pattern.
- Node generated JS/declaration artifacts derive from source, so the source
  export must be deleted and regenerated rather than hidden by editing build
  output.
- The Node cross-language verifier fixture must build normalized authority
  proof facts directly. Borrowing `authorityProof` from an empty helper keeps
  the old proof model alive even when the surrounding proof-fact fields are
  non-empty.
- The excluded Node TypeScript authority-anchor test is obsolete because the
  active `tests/receipt-authority-anchor.test.mjs` suite already pins the
  shared `axon-receipt-anchor-v2` anchors. Keeping the excluded test would
  preserve contradictory receipt evidence.
- The V2 RF-6 gate now rejects `EMPTY_RECEIPT_PROOF_FACTS` anywhere under Node
  SDK source or tests because public API manifest checks cannot see generated
  build artifacts or excluded fixture files reliably.
- Python `ReceiptProofFacts` default field values are an omitted proof-facts
  constructor path. Removing only runtime calls is insufficient while
  `ReceiptProofFacts()` remains valid for tests, examples, or downstream
  callers.
- Python tests and examples must construct receipt proof facts at the receipt
  signing boundary. Fixture helpers may centralize the construction, but they
  must still pass explicit subject, descriptor, runtime, authority, hash, and
  parent-receipt fields.
- Existing cross-language authority anchors are shared fixture evidence, not a
  Python-only decision point. The Python anchor test therefore keeps the
  current `axon-receipt-anchor-v2` values as explicit proof facts instead of
  changing anchor constants before the Rust/Go/Node/Java/Swift anchor model is
  migrated together.
- The V2 RF-6 gate now uses a Python AST scan for empty
  `ReceiptProofFacts()` calls across Python SDK source, tests, and examples
  because grep patterns can miss multiline empty calls or non-runtime fixture
  regressions.
- Rust `ReceiptProofFacts: Default` is an RF-6 constructor defect. It makes an
  empty receipt proof object a first-class lifecycle state, even if later
  normalizers patch the fields before emission.
- `InvocationCore::new_with_policy` keeps source-compatible behavior by
  deriving complete local proof facts from `AxiomBinding`; it no longer uses
  an empty receipt-facts object as the default construction path.
- Descriptor-bound LocalRuntime normalization is now an explicit two-state
  model. Runtime-owned omitted facts are derived directly from the admitted
  envelope and registered descriptor proof binding; caller-supplied facts are
  treated as a complete object and rejected if descriptor version or subject
  ref is missing.
- Rust test fixtures that need zero hash predicate coverage should set zero
  hash fields on an otherwise explicit receipt proof-facts object. They must
  not use `Default` as a shortcut for an invalid receipt proof model.
- The V2 RF-6 gate now rejects Rust `ReceiptProofFacts::default()`,
  `proof_facts: Default::default()`, and `Default` derive on
  `ReceiptProofFacts` because public API manifests cannot reliably detect
  derive-based constructor semantics or test/example defaults.
- Python `InvocationAuthorityProof` defaults are a nested RF-6 defect. A
  receipt proof-facts object is not complete if its authority proof was
  created by omitted default fields.
- Python authority-proof call sites now pass empty payload and absent
  signature explicitly. This keeps the receipt authority model readable at
  the signing boundary instead of hiding optionality in dataclass defaults.
- The shared authority-anchor fixture may still encode an empty authority
  proof while the cross-language anchor model is open, but it must do so as
  an explicit fixture value and not through `InvocationAuthorityProof`
  constructor defaults.
- The V2 RF-6 gate now requires Python `InvocationAuthorityProof(...)` calls
  in SDK source, tests, and examples to use the full named field set. That
  catches partial omitted-authority proofs, not only zero-argument calls.
- Node `EMPTY_AUTHORITY_PROOF` is a public omitted-authority construction
  value, not a harmless fixture. Keeping it would preserve RF-6 as an SDK
  integration pattern even after receipt proof facts became explicit.
- The Node receipt-authority anchor suite may retain the shared empty
  authority proof while the cross-language anchor model is open, but that
  value belongs as a file-local fixture rather than an exported SDK helper.
- The V2 RF-6 gate now rejects `EMPTY_AUTHORITY_PROOF` anywhere under Node SDK
  source or tests because generated JS/declaration artifacts and excluded
  fixtures can otherwise keep the wrong authority model alive.
- Java `InvocationAuthorityProof.empty()` is the same RF-6 defect as Node's
  removed empty authority helper. It is a public SDK factory for omitted
  authority proof facts, not a harmless convenience constructor.
- Java examples are part of the effective SDK contract. The receipt-closure
  example therefore keeps its shared anchor authority proof as a file-local
  explicit fixture instead of importing a canonical empty helper.
- Java tests may centralize the current empty authority anchor as a
  package-private fixture, but the construction must remain explicit and
  outside the production SDK surface.
- The V2 RF-6 gate now rejects `InvocationAuthorityProof.empty()` anywhere
  under Java SDK source, examples, or tests because public manifest checks do
  not catch semantic helper factories embedded inside nested Java classes.
- Swift `InvocationAuthorityProof.empty` and defaulted initializer parameters
  are one RF-6 defect. Removing only `.empty` would still allow omitted
  authority proof facts through partial constructor calls.
- Swift `InvocationAuthorityProof` construction now mirrors the strict Python
  authority-proof constructor: proof type, binding, payload, hash, issuer,
  signature, and admission hook must all be spelled out by the caller.
- Swift LocalRuntime may normalize a zero proof hash to the expected binding
  hash, but it may not rely on defaulted proof payload, issuer, signature, or
  admission hook values at the constructor boundary.
- The Swift shared authority-anchor suite may retain an empty authority proof
  while the cross-language anchor model is open, but that value belongs as a
  test-local explicit fixture rather than a public SDK singleton.
- The V2 RF-6 gate now rejects Swift empty authority helpers and defaulted
  authority-proof initializer parameters because source scans are the only
  reliable way to catch default-argument semantics in Swift public APIs.
- Go `InvocationAuthorityProof{}` in receipt fixtures is an omitted
  authority-proof construction path even though Go cannot prohibit all
  zero-value structs at the type level.
- The Go shared anchor may still encode an empty authority proof while the
  cross-language anchor model is open, but the fixture must list every field
  explicitly through `anchorAuthorityProof()`.
- Go `bundle.go` may keep zero-value `InvocationAuthorityProof{}` in error
  returns because those values are discarded alongside non-nil errors; the V2
  gate therefore targets active invocation package construction sites outside
  that decode-return path.
- Rust `InvocationAuthorityProof: Default` is an RF-6 defect for the same
  reason as Swift defaulted initializer parameters: it lets callers omit the
  authority proof tail while constructing complete-looking receipt facts.
- Rust LocalRuntime may still normalize a zero authority proof hash, but that
  zero hash is now an explicit constructor argument rather than a side effect
  of `Default`.
- Rust shared anchors may continue to encode an empty authority proof while
  the cross-language anchor model is open, but they must call
  `InvocationAuthorityProof::new("", None, Vec::new(), [0; 32], None, None,
  "")` explicitly.
- The V2 RF-6 gate now rejects Rust authority-proof Default derive/calls and
  receipt-proof struct update defaults because those patterns can survive
  public API manifest scans while keeping omitted proof facts in active tests.
- The runtime client SDK protobuf adapter is not allowed to keep a weaker
  proof-fact model than the canonical Rust SDK. `ReceiptProofFacts` in that
  adapter is transport data, but it still participates in receipt signing and
  verification, so optional authority proof is a canonical proof fork.
- Missing authority proof in an admission receipt is now a fail-closed
  terminal receipt construction error, not an instruction to synthesize an
  empty canonical proof.
- `easynet-verify` must consume the same required proof-fact shape as the
  runtime client adapter because offline verification is part of the receipt
  trust boundary, not a separate compatibility model.
- The V2 RF-6 gate now includes `core/runtime-rs/client-sdk` receipt adapter
  scans because SDK-only Rust scans do not catch duplicate same-language proof
  DTOs in the runtime transport package.
- Private Rust legacy plain proof helpers are still architecture, not harmless
  implementation detail. A same-module plain encoder, signer, verifier, and
  admission path preserves a second proof model even when public exports have
  moved to descriptor-bound bytes.
- Rust invocation tests should sign the same canonical material that
  production admission verifies: `DescriptorBoundEnvelope` canonical bytes.
  Historical arbitrary-string fixture behavior is removed rather than
  simulated behind a compatibility helper.
- Descriptor-bound tests must use valid runtime URAs for caller, callee,
  subject, and ability descriptor refs. Invalid arbitrary URA strings are no
  longer useful fixture data once the canonical proof boundary validates the
  tuple before signing or verification.
- The V2 RF-3 gate now rejects Rust legacy plain helper names in invocation
  source. Public-manifest checks alone are insufficient because private
  same-module helpers can still preserve and spread the obsolete proof model.
- Package-private Java methods are still production architecture. Keeping
  `legacyPlainInvocationBytes`, legacy plain signing, legacy plain verifier,
  and legacy plain admission in Java production source preserves a second
  proof model even when package-root exports have moved to descriptor-bound
  helpers.
- Java axiom vectors already carry descriptor-ref ability values, so their
  Java driver should exercise descriptor-bound canonical bytes directly. Using
  legacy plain bytes to consume those vectors hid the very RF-3 cutover the
  vectors are meant to prove.
- Java admission tests should enter `runDescriptorBoundAdmission` because that
  is the runtime admission boundary used by `LocalRuntime`. A test-only
  `runLegacyPlainAdmission` runner would be a compatibility path, not a
  state-machine proof.
- The V2 RF-3 gate now rejects Java production legacy plain helper names in
  addition to public static plain helper names because Java access modifiers do
  not make canonical-domain duplicate proof models harmless.
- Python underscore-prefixed helpers are still production package
  architecture when they live under `easynet_axon.invocation`. Keeping
  `_legacy_plain_invocation_bytes`, legacy plain signing, legacy plain
  verifier, and `_run_legacy_plain_admission` preserves a second proof and
  admission model even though those names are private by convention.
- Python axiom-vector drivers should exercise descriptor-bound canonical bytes
  for relational signature and tuple-binding properties. The shared vectors
  already carry descriptor refs, so plain-byte verification is obsolete test
  architecture rather than necessary fixture coverage.
- Python full-suite industrial lifecycle failures are RF-4 evidence, not RF-3
  proof-boundary failures. They should be addressed by lifecycle facade
  convergence rather than by retaining legacy plain admission helpers.
- The V2 RF-3 gate now rejects Python legacy plain helper names across source,
  tests, and examples because a private helper imported by tests is enough to
  keep the retired proof model live.
- Internal Swift functions are still canonical-domain architecture when they
  live under `EasyNetAxon/Invocation`. Keeping `legacyPlainInvocationBytes`,
  legacy plain signing, legacy plain verifier, and `runLegacyPlainAdmission`
  would preserve a second proof/admission model even without public exports.
- Swift admission tests should exercise `runDescriptorBoundAdmission` because
  it is the runtime state-machine boundary. A legacy plain admission runner in
  tests would be a compatibility path and would weaken RF-3 convergence.
- Swift axiom-vector tests should consume descriptor-bound canonical bytes for
  tuple-binding properties. The shared vectors already use descriptor-ref
  ability strings, so a plain encoder is not required to keep those vector
  invariants meaningful.
- The V2 RF-3 gate now rejects Swift production legacy plain helper names in
  addition to public plain helper names because source-level absence is the
  clean target for proof/admission convergence.
- Go test files inside `easynet/invocation` are still part of the SDK proof
  architecture. A test-scoped `legacy_plain_fixtures_test.go` encoder and
  signer can be copied into production later and therefore preserves the
  retired proof model.
- Go axiom tests should use valid descriptor refs and subject URAs once they
  exercise descriptor-bound bytes. Invalid arbitrary strings only made sense
  when the tests bypassed descriptor-bound validation.
- The V2 RF-3 gate now rejects Go legacy plain helper names across the whole
  invocation package, including tests, because old vector fixture
  compatibility is not a clean-target proof model.
- Node vector scripts are SDK proof architecture when they sign and verify
  conformance vectors. Keeping a standalone legacy plain fixture script would
  preserve the retired proof model even after production source converges.
- Node axiom-vector tests and vector runners should consume the same
  descriptor-bound helpers exported by the runtime package. Historical plain
  vector compatibility is not a reason to keep a second signer/verifier.
- The V2 RF-3 gate now rejects Node legacy plain helper names across the full
  Node SDK package, excluding only dependency folders, because script/test
  fixtures are enough to keep obsolete proof semantics live.
- Active RFCs, checklists, conformance metadata, and SDK comments are part of
  the canonical architecture surface. If they keep names such as
  `verify_signature`, `canonical_invocation_bytes`, or `caller.uri`, they can
  recreate the same root fork after the implementation has been cleaned.
- Descriptor-bound proof terminology must be used in active specifications
  even when a document is describing a negative boundary. The clean target is
  to name the canonical proof model directly and avoid preserving obsolete
  helper names as normal vocabulary.
- URA terminology gates should include active SDK interface documents, not
  only RFC prose. Interface examples with `"uri"` fields or
  `envelope.caller.uri` teach downstream SDK contracts and therefore belong in
  the same RF-9 active-document gate.
- Active ontology documents are stronger than ordinary prose. If an ontology
  file is not explicitly historical, pseudo-types such as `AgentUri` define
  architecture vocabulary and must converge to `AgentURA`.
- Axiom conformance README/vector descriptions are part of the test contract.
  Identity examples in those files must say URA because test authors copy
  their vocabulary into SDK assertions and driver names.
- Endpoint adapter tests should use endpoint naming when the tested value is a
  transport scheme. That keeps the URA rule focused on routable identity
  architecture while avoiding retired address-token vocabulary in test names.
- Axon `document/` files should be treated as active vocabulary unless a
  future classification explicitly marks them historical. A manually curated
  active-document list is too weak for RF-9 because new documents can drift
  outside the list.
- When `axon://` appears in active text, describe it as an endpoint if that is
  the tested or packaged object. `easynet:///` identity examples remain URAs.
- Brand and strategy documents can still affect architecture by shaping
  vocabulary. They should not preserve retired address terms merely because
  they are not implementation files.
- Dendrite bridge contract docs under `core/runtime-rs` are active caller
  contracts even though they are outside Axon's top-level `document/` tree.
  RF-9 gates must cover them explicitly when they define SDK-facing shapes.
- Legacy FFI compatibility should be described as an edge adapter pending
  descriptor-bound migration, not as a permanent canonical proof path.
- Test names are vocabulary too. A test that checks a URA field should say
  URA; a test that checks a transport scheme should say endpoint.
- RF-9 active source gates should scan repository roots, not only paths where
  a previous audit found issues. Otherwise new source files can preserve
  retired terminology outside the gate.
- Build outputs, virtual environments, package metadata, and caches are not
  canonical source. The gate should exclude them rather than force generated
  third-party files to follow EasyNet terminology.
- `http::uri::PathAndQuery` and similar imports are transport-library API
  names. They are allowed because they do not describe routable Axon identity.
- Schema-source self-tests must exercise Axon's real proto syncer instead of a
  fake pass/fail shim. RF-9 is about ownership of canonical source,
  derivation, catalog parity, and codegen compatibility, so the regression
  test must mutate those concrete surfaces.
- EasyNet-Cli may verify schema-source convergence, but it must not grow a
  second schema derivation model. The syncer, canonical filename set, product
  proto boundary, Dendrite catalog parity, and codegen version rules remain
  Axon-owned.
- Benchmark coverage for V2 acceptance must use executable runtime scenarios,
  not prose-only claims. Stream, bidi, and cancellation cleanup baselines belong
  in the Axon `LocalRuntime` harness because Axon owns the generic runtime
  state machine.
- Benchmarks that need terminal receipts must install an explicit receipt
  signing provider. The fail-closed `LocalRuntime::new()` constructor is a seam
  and should not be silently promoted into provider-backed benchmark evidence.
- Allocation baseline coverage remains open until an allocator-counting harness
  exists. Timing-only Criterion rows must not be used as evidence for
  allocation regressions.
- Allocation baselines should be measured by an allocator-counting harness, not
  by encoding counts as fake time in Criterion. Timing and allocation are
  distinct acceptance surfaces.
- Benchmark runtime setup is shared through a bench-local support module so
  latency and allocation rows cannot quietly diverge through different receipt
  authority or descriptor-bound fixture construction.
- A registered direct runtime provider plus closed live case bindings is
  stronger than API-shape parity and must be reflected as `ProviderBacked` in
  the canonical matrix. Leaving those cells as `Seam` hides real provider
  state and lets RF-4 drift without an explicit proof boundary.
- Provider step evidence is action-closed, not selector-unique. A single live
  conformance test may exercise all actions in a case; the matrix must bind
  every action to that selector instead of inventing fake per-action selectors.
- Go/Python `native_runtime`, `unary_invoke`, and `stream` can be marked
  `ProviderBacked` because their registered direct runtime providers have
  implementation digests and all required case selectors are live. They are
  not `CutoverReady` because full transition-vector and recovery cutover is
  still open.
- `bidi` must remain `Seam` while `bidi/frame0_required` is unproven. Positive
  bidi close/backpressure evidence is not enough to claim provider-backed
  lifecycle parity for the full bidi contract.
- Lifecycle parity needs a canonical state-machine contract, not only an
  action-name list. The conformance source manifest now owns the transition
  contract, and the generated matrix carries the same contract so facades do
  not invent language-specific lifecycle semantics.
- The lifecycle transition contract names allowed source states and exactly one
  next or terminal state per action. Deadline ownership, child-deadline
  propagation, cancellation acknowledgement, idempotent replay, bounded
  queue/concurrency behavior, cleanup responsibility, and receipt/event
  observability are first-class fields rather than prose-only claims.
- Start and restart recovery are runtime-host lifecycle actions, while
  dispatch, stream, bidi, child dispatch, cancel, deadline, and terminal
  receipt are invocation/session lifecycle actions. Keeping both in one
  contract is intentional because RF-4 requires one shared runtime model across
  facades.
- Adding the transition contract does not make any language `CutoverReady`.
  Cutover still requires executable transition/recovery vectors and public
  error-contract proof against the same provider/runtime version.
