# Canonical Runtime Convergence V2 - Execution Checklist

## Descriptor Projection Slice

- [x] Identify the loose descriptor/hash input boundary.
- [x] Replace the scatter-argument summary function with a semantic projection
      object.
- [x] Migrate the sole production caller.
- [x] Remove the obsolete public scatter-argument helper.
- [x] Verify descriptor tests and lib build.
- [ ] Close remaining SPEC root forks RF-1 through RF-9.

## Mission Terminal Transition Slice

- [x] Identify mission terminal constructors that encoded lifecycle facts as
      long positional parameter lists.
- [x] Split terminal transition input into run context, completion facts, and
      failure facts.
- [x] Preserve the existing `running -> terminal` aggregate transition and
      terminal immutability rule.
- [x] Migrate mission run completion and failure callers.
- [x] Verify mission orchestration tests and lib build.

## Kernel Default Lifecycle Slice

- [x] Identify the daemon kernel default lifecycle constructor.
- [x] Add `Default` as a standard object lifecycle entry that delegates to
      `Kernel::new()`.
- [x] Preserve `new_with_subscriber_broker()` as an explicit daemon boot
      policy constructor.
- [x] Verify kernel tests and lib build.

## Bidi Event Payload Ownership Slice

- [x] Identify stream/bidi lifecycle event enums whose large payload variants
      inflated every queued event.
- [x] Box local bidi forwarded down-frames at the handler event boundary.
- [x] Box carrier-v1 admission and terminal events at classification output.
- [x] Box pending stream admission events to match existing boxed terminal
      results.
- [x] Migrate dispatchers and tests without changing admission/chunk/terminal
      ordering semantics.
- [x] Verify bidi and service-bidi tests plus lib build.

## Session Escalation Reply Ownership Slice

- [x] Identify reverse-session escalation reply variants that inflated every
      correlation slot.
- [x] Box canonical `InvokeResponse` replies while preserving one-reply
      correlation semantics.
- [x] Name ready-hook callback and hook-list types at the shared session outbox
      boundary.
- [x] Verify session escalation and local session dispatcher tests plus lib
      build.

## Dispatch Result Projection Slice

- [x] Identify carrier result construction sites with implicit default tails or
      obfuscated terminal payload projection.
- [x] Replace terminal payload selection with an explicit completion branch.
- [x] Remove no-op default tails from fully specified carrier
      `DispatchResult` literals.
- [x] Verify local session dispatcher and Axon bridge dispatch shim tests plus
      lib build.

## Resolver Ingress Tuple Source Slice

- [x] Identify resolver-layer tuple ingress that encoded public/system source
      as `Option<subject>` plus implicit root causal context.
- [x] Add `InvocationPlanIngress` as a closed source-state enum.
- [x] Map daemon-system and public-ingress sources to explicit target subject
      and causal-context bindings.
- [x] Validate public-ingress subject URA before target construction.
- [x] Add positive and negative resolver tests for public ingress tuple
      propagation.

## Invocation Target Construction Boundary Slice

- [x] Add named `InvocationTarget` constructors for local daemon-system and
      explicit-tuple dispatch states.
- [x] Add constructor tests that pin subject and causal-context projection.
- [x] Migrate clean production adapter call sites in agent discover/invoke and
      MCP/A2A/OpenAI compatibility integrations.
- [x] Leave remaining direct target literals visible as RF-8/RF-7 migration
      inventory instead of claiming closure.

## Plugin Host Target Test Boundary Slice

- [x] Identify plugin host tests that still hand-assembled local
      `InvocationTarget` literals.
- [x] Replace declarative plugin test targets with named routing target
      constructors.
- [x] Verify plugin host tests and lib build.
- [x] Confirm `host_api.rs` has no remaining direct `InvocationTarget`
      literal.

## Resource and Governance Target Boundary Slice

- [x] Identify clean resource/governance target literals outside the current
      dirty worktree.
- [x] Migrate page API ability forwarding to
      `InvocationTarget::local_daemon_system_with_subject`.
- [x] Migrate governance health local smoke target to
      `InvocationTarget::local_daemon_system`.
- [x] Verify focused page API and governance health tests plus lib build.

## Media Subject Target Fixture Slice

- [x] Identify mic and screen media tests that still hand-assembled local
      `InvocationTarget` literals.
- [x] Migrate explicit media resource subjects to
      `InvocationTarget::local_daemon_system_with_subject`.
- [x] Migrate missing-subject screen fixture to
      `InvocationTarget::local_daemon_system`.
- [x] Verify focused mic and screen media tests plus lib build.
- [x] Leave camera media fixtures visible as remaining RF-8/RF-7 inventory.

## Camera Subject Target Fixture Slice

- [x] Identify camera media tests that still hand-assembled local
      `InvocationTarget` literals.
- [x] Migrate snapshot, subscribe, record start, and record stop targets with
      explicit resource subjects to
      `InvocationTarget::local_daemon_system_with_subject`.
- [x] Migrate missing-subject camera targets to
      `InvocationTarget::local_daemon_system`.
- [x] Verify focused camera media tests plus lib build.
- [x] Confirm `camera_snapshot.rs` has no remaining direct target literal or
      target enum construction.

## LocalRuntime Subject Derivation Ownership Slice

- [x] Identify `local_runtime_invoker` as a second descriptor-default subject
      derivation owner.
- [x] Move subject URA resolution onto `InvocationTarget`.
- [x] Preserve explicit subject validation and daemon-system descriptor
      subject policy as target-domain behavior.
- [x] Make `local_runtime_invoker` consume resolved subject and causal context
      instead of defining a parallel subject policy enum.
- [x] Verify routing target tests, LocalRuntime invoker tests, lib build, and
      architecture convergence gate.

## Mission Catalog Gateway Target Boundary Slice

- [x] Identify the cfg-test Mission catalog gateway as a remaining direct
      local `InvocationTarget` assembly point.
- [x] Preserve production Mission child dispatch through admitted parent
      `AbilityContext` and `ChildInvocationRequest`.
- [x] Migrate the test catalog gateway to
      `InvocationTarget::local_daemon_system`.
- [x] Verify Mission invocation gateway tests, lib build, architecture
      convergence gate, and absence of target literal remnants in the file.

## Ability Dispatch Target Fixture Boundary Slice

- [x] Identify `AxonAbilityCatalog` tests that still hand-assembled local and
      remote `InvocationTarget` fixtures.
- [x] Add `InvocationTarget::remote_daemon_system` so remote guard fixtures do
      not restate daemon-system subject and root-causal policy.
- [x] Reuse the target value object's scoped binding constructor for local and
      remote daemon-system targets.
- [x] Migrate RPC, stream, bidi, explicit-subject, and remote guard fixtures
      in `dispatch.rs` to named target constructors.
- [x] Verify routing target tests, ability dispatch tests, lib build,
      architecture convergence gate, and absence of dispatch target literals.

## LocalRuntime Invoker Target Fixture Boundary Slice

- [x] Identify `local_runtime_invoker` tests that still hand-assembled local
      `InvocationTarget` fixtures.
- [x] Preserve production remote rejection through `TargetScope` matching.
- [x] Migrate explicit-subject and daemon-system test helper paths to
      `InvocationTarget` constructors.
- [x] Verify LocalRuntime invoker tests, routing target tests, lib build,
      architecture convergence gate, and absence of target literal remnants in
      `local_runtime_invoker.rs`.

## Builtins Smoke Target Fixture Boundary Slice

- [x] Identify broad built-in smoke and catalog assembly fixtures that still
      hand-assembled local `InvocationTarget` values.
- [x] Migrate `real_invoke_tests` shared target helper to
      `InvocationTarget::local_daemon_system`.
- [x] Preserve per-test subject and metadata overrides through target builder
      methods.
- [x] Migrate catalog assembly read-only smoke dispatch targets to
      `InvocationTarget::local_daemon_system`.
- [x] Verify real-invoke tests, catalog assembly tests, lib build, architecture
      convergence gate, and absence of target literal remnants in the migrated
      files.

## CLI Agent Command Target Fixture Boundary Slice

- [x] Identify the CLI agent command fixture as a remaining local
      `InvocationTarget` assembly point.
- [x] Preserve the explicit envelope-aware handler branch as an
      `EnvelopeContext` test path.
- [x] Migrate ordinary agent command fixture dispatch to
      `InvocationTarget::local_daemon_system`.
- [x] Verify focused CLI agent command tests, lib build, architecture
      convergence gate, and absence of target literal remnants in the file.

## Protobuf Transport Target Projection Boundary Slice

- [x] Identify remaining crate-internal `EnvelopeOpen.target` protobuf target
      literals in SDK bidi frame construction, `session.open`, local daemon
      gRPC bidi invocation, and service test helpers.
- [x] Add `invocation_wire::wire_invocation_target` as the single daemon-owned
      protobuf target selector projector.
- [x] Reject empty target selectors before constructing bidi frame-0 wire
      messages.
- [x] Migrate production and crate-internal test helpers to the named wire
      projector.
- [x] Leave external integration raw protobuf fixtures as external-client
      input, not as internal construction paths.
- [x] Verify focused target projector and bidi helper tests, lib build,
      architecture convergence gate, and absence of crate-internal
      `target: Some(InvocationTarget { ... })` literals.

## RF-5 Rust Public Surface Signer Fallback Removal Slice

- [x] Identify `generate_subject_auth` and
      `runtime_admin.generate_subject_auth` as Rust public-surface evidence
      still counted in the canonical SDK capability graph.
- [x] Remove `GeneratedSubjectAuth`, `generate_subject_auth`,
      `generate_private_agent_auth`, and `generate_private_hub_auth` from the
      Axon Rust SDK runtime-admin public surface.
- [x] Replace the generated-auth unit test with a pure subject-identifier
      helper test so runtime-admin no longer validates process-local secret
      generation.
- [x] Extend public-surface policy to classify generated/default subject auth,
      generated private agent/hub auth, process-local signer, and private-key
      authenticator symbols as RF-5 non-canonical signer fallback defects.
- [x] Extend the V2 manifest gate to reject fallback signer helpers in
      canonical language/member graphs.
- [x] Regenerate the canonical public API manifest and SDK parity matrix with
      the generated auth group absent.
- [x] Verify Axon runtime-admin tests, V2 convergence script, conformance
      policy self-test, architecture convergence gates, formatting, and lib
      build.
- [x] Record that cross-language signer-handle/KeyService convergence remains
      open; this slice removes the Rust public fallback root but is not the
      full RF-5 cutover.

## RF-3 Public Plain Proof Helper Removal Slice

- [x] Identify Rust package-root exports for plain canonical/admission helpers:
      `canonical_invocation_bytes`, `sign_invocation`,
      `verify_invocation_signature`, `verify_phase`, `verify_signature`, and
      `run_admission`.
- [x] Remove plain helper re-exports from the Axon Rust invocation package
      root.
- [x] Restrict the underlying Rust plain helpers to crate-internal test-only
      functions so they disappear from rustdoc public inventory.
- [x] Migrate runtime-admin resolver tests to descriptor-bound signing and
      verification.
- [x] Remove plain helper exports from the Axon Python invocation package root
      and expose descriptor-bound admission replacements.
- [x] Extend EasyNet-Cli conformance policy/self-tests to classify the complete
      plain helper group as RF-3 defects.
- [x] Upgrade the V2 convergence gate so plain helpers fail even when they
      appear only in legacy quarantine.
- [x] Regenerate public API artifacts and verify exact absence from manifest
      and parity matrix.
- [x] Verify Axon admission/axiom/runtime-admin tests, Python public-surface
      smoke, V2 gate/self-test, EasyNet-Cli formatting, and lib build.

## RF-3 Python Submodule Plain Proof Hardening Slice

- [x] Identify that Python package-root cleanup still left plain helpers as
      non-underscore submodule functions in `axiom.py` and `admission.py`.
- [x] Rename Python plain helper functions to private fixture names:
      `_canonical_invocation_bytes`, `_sign_invocation`,
      `_verify_invocation_signature`, `_verify_signature`, and
      `_run_admission`.
- [x] Remove unused plain helper imports from Python runtime admission tests.
- [x] Migrate historical axiom vector and cross-language bundle tests to call
      the private fixture names explicitly.
- [x] Add a direct Axon source-level V2 gate for public Rust/Python plain
      proof/admission helper definitions and re-exports.
- [x] Add a self-test fixture proving the new source-level gate fails when an
      Axon Python submodule exposes a public plain proof helper.
- [x] Verify Python public-surface smoke, repo-managed pytest for admission and
      axiom vectors, Axon Rust format/check, and V2 gate/self-test.

## RF-6 Java LocalRuntime Receipt Proof Facts Slice

- [x] Identify that Java receipt constructors reject omitted proof facts but
      production `LocalRuntime` still emitted bindings with
      `ReceiptProofFacts.empty()`.
- [x] Add immutable per-event proof-fact output-hash replacement on
      `InvocationReceipt.AxiomBinding`.
- [x] Move Java LocalRuntime signed descriptor-bound receipt facts to a local
      proof normalizer at the admission binding boundary.
- [x] Give system-local `invokeAsync` receipts a separate
      `system-local.invoke.v1` proof identity rather than reusing empty facts.
- [x] Add Java behavior tests proving signed and system-local terminal receipts
      carry non-empty schema/impl hashes, authority proof, runtime env, input
      hash, and output hash.
- [x] Extend the EasyNet-Cli V2 gate and self-test to reject
      `ReceiptProofFacts.empty()` in Java `LocalRuntime`.
- [x] Verify targeted Java invocation tests plus V2 gate/self-test.

## RF-6 Python LocalRuntime Receipt Proof Facts Slice

- [x] Identify that Python `LocalRuntime` still emitted signed and
      system-local bindings with default `ReceiptProofFacts()`.
- [x] Add `ReceiptProofFacts.with_output_hash` so event receipts can refresh
      proof output facts without mutating the admission binding.
- [x] Refresh `_InvocationCore.emit` per-event proof facts alongside
      per-event payload digest.
- [x] Move Python signed descriptor-bound receipt facts to
      `_LocalReceiptProofFacts` at the admission binding boundary.
- [x] Give Python system-local `invoke_async` receipts the same separate
      `system-local.invoke.v1` proof identity used for Java.
- [x] Add Python runtime tests proving signed and system-local terminal
      receipts carry non-empty schema/impl hashes, authority proof, runtime
      env, input hash, and output hash.
- [x] Extend the EasyNet-Cli V2 gate and self-test to reject
      `proof_facts=ReceiptProofFacts()` in Python `LocalRuntime`.
- [x] Verify Python admission tests plus V2 gate/self-test.

## RF-6 Go LocalRuntime Receipt Proof Facts Slice

- [x] Identify that Go `LocalRuntime` still emitted signed and system-local
      bindings with `EmptyReceiptProofFacts()`.
- [x] Add `ReceiptProofFacts.WithOutputHash` so event receipts can refresh
      proof output facts without mutating the admission binding.
- [x] Refresh `InvocationCore.emit` per-event proof facts alongside per-event
      payload digest.
- [x] Move Go signed descriptor-bound receipt facts to the LocalRuntime
      binding boundary.
- [x] Converge Go `AbilityDescriptorRef` parsing on the Rust canonical
      `ability_ura@version#descriptor_hash!admission_action` shape.
- [x] Migrate Go cross-language bundle fixtures to descriptor-bound signing
      and digest/action-bound ability refs.
- [x] Give Go system-local `InvokeAsync` receipts the same separate
      `system-local.invoke.v1` proof identity used for Java and Python.
- [x] Add Go runtime tests proving signed and system-local terminal receipts
      carry non-empty schema/impl hashes, authority proof, runtime env, input
      hash, and output hash.
- [x] Extend the EasyNet-Cli V2 gate and self-test to reject
      `EmptyReceiptProofFacts()` in Go `LocalRuntime`.
- [x] Verify Go invocation tests plus V2 gate/self-test.

## RF-4 Go Runtime Lifecycle Facade Slice

- [x] Identify that Go industrial lifecycle vectors required runtime-level
      `CoreOf`, `Cancel`, and `SendMessage` APIs.
- [x] Preserve a single lifecycle owner by delegating runtime-level
      `Cancel` and `SendMessage` through the existing generation-checked
      control state machine.
- [x] Expose `CoreOf` as an inspection surface for immutable snapshots and
      current-state queries.
- [x] Verify Go invocation package tests, focused industrial lifecycle/audit
      vectors, and the full Go industrial package.

## RF-6 Node LocalRuntime Receipt Proof Facts Slice

- [x] Identify that Node `LocalRuntime` still emitted signed and system-local
      bindings with `EMPTY_RECEIPT_PROOF_FACTS`.
- [x] Add `receiptProofFactsWithOutputHash` so event receipts can refresh
      proof output facts without mutating the admission binding.
- [x] Refresh `InvocationCore.emit` per-event proof facts alongside per-event
      payload digest.
- [x] Move Node signed descriptor-bound receipt facts to the LocalRuntime
      binding boundary.
- [x] Give Node system-local `invokeAsync` receipts the same separate
      `system-local.invoke.v1` proof identity used for Java, Python, and Go.
- [x] Add Node runtime tests proving signed and system-local terminal receipts
      carry non-empty schema/impl hashes, authority proof, runtime env, input
      hash, and output hash.
- [x] Extend the EasyNet-Cli V2 gate and self-test to reject
      `EMPTY_RECEIPT_PROOF_FACTS` in Node `LocalRuntime`.
- [x] Verify Node invocation tests, Node package verify, and V2
      gate/self-test.

## RF-5 Rust Local-Fast Signer Feature Removal Slice

- [x] Identify `local-fast-probes` as a public Rust SDK feature that exposed
      process-local signer fallback helpers outside crate tests.
- [x] Remove the Cargo feature and all Rust SDK `feature = "local-fast-probes"`
      cfg gates.
- [x] Restrict local-fast constructors and fallback signer helpers to
      crate-internal `cfg(test)` use.
- [x] Move integration tests to explicit descriptor-bound test providers
      owned by `tests/common/descriptor_bound_support.rs`.
- [x] Move the `receipt_closure` example to an explicit receipt signing
      authority provider instead of `new_local_fast`.
- [x] Remove EasyNet-Cli's downstream `local-fast-probes` feature and Axon
      dev-dependency feature request.
- [x] Migrate `real-user-smoke` and the Pages integration runtime fixture away
      from `LocalRuntime::new_local_fast`.
- [x] Extend the EasyNet-Cli V2 gate and self-test to reject reintroduced
      `local-fast-probes` public feature/cfg and external helper consumption.
- [x] Extend the V2 gate and self-test to reject EasyNet-Cli downstream
      reintroduction of the deleted feature or old local-fast helper
      consumption.
- [x] Verify Rust SDK checks, targeted tests, example execution, full test
      target compilation, and V2 gate/self-test.
- [x] Record that RF-5 remains open for cross-language signer-handle parity
      and daemon KeyService authority cutover.

## RF-5 Runtime Client Subject Auth Generator Removal Slice

- [x] Identify `AxonClient::generate_subject_auth` in
      `core/runtime-rs/client-sdk` as a public process-local signing material
      generator.
- [x] Remove the generator instead of retaining a compatibility wrapper.
- [x] Keep `EasyNetUserAuth` only as an explicit host-supplied signing input
      for current authenticated call paths.
- [x] Migrate runtime client SDK tests to a local fixed `host_auth_fixture`.
- [x] Remove the test that asserted fresh SDK-generated subject secrets.
- [x] Extend the V2 gate and self-test with a source-level Axon process-local
      signer fallback scan covering runtime client SDK, canonical SDK
      packages, and runtime source.
- [x] Verify runtime client SDK check/test, fallback source scan, and V2
      gate/self-test.
- [x] Record that RF-5 remains open for signer-handle or daemon KeyService
      authority convergence across language facades.

## RF-3 Go Public Plain Proof Helper Removal Slice

- [x] Identify Go exported plain proof/admission helpers:
      `CanonicalInvocationBytes`, `SignInvocation`,
      `VerifyInvocationSignature`, `VerifySignature`, and `RunAdmission`.
- [x] Rename the Go plain helper group to package-private functions while
      preserving package-local historical vector tests.
- [x] Preserve exported descriptor-bound Go proof/admission replacements.
- [x] Update `sdk/API_MAPPING.md` to document descriptor-bound proof names
      instead of plain proof names.
- [x] Extend the V2 gate and self-test to reject Go public plain helper
      reintroduction.
- [x] Verify Go invocation package tests, public-symbol scan, and V2
      gate/self-test.
- [x] Record that RF-3 remains open for remaining language surfaces and
      examples/vector audit.

## Still Required Before Completion

- [ ] RF-5 cross-language signer-handle and daemon KeyService convergence.
- [ ] RF-3 remaining language package/vector/example audit for
      descriptor-bound-only public proof.
- [ ] RF-8/RF-7 complete tuple ingress and LocalRuntime-only daemon routes.
- [ ] RF-4 shared lifecycle matrix, transition vectors, and cross-language
      provider status cutover.
- [ ] RF-6 remaining examples and tests, and descriptor proof-binding parity
      closure.
- [ ] RF-1 product SDK surface extraction.
- [ ] RF-2 Mission/EAL extraction from Axon canonical runtime.
- [ ] RF-9 URA terminology and generated-schema ownership closure.
