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
