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
