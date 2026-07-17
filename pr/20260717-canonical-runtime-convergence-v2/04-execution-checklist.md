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

## RF-3 Swift Public Plain Proof Helper Removal Slice

- [x] Identify Swift public plain proof/admission helpers:
      `canonicalInvocationBytes`, `signInvocation`,
      `verifyInvocationSignature`, `verifySignature`, and `runAdmission`.
- [x] Rename Swift plain helper logic to internal `legacyPlain*` fixture
      functions.
- [x] Migrate Swift historical axiom/admission tests to the internal legacy
      fixture names through `@testable import`.
- [x] Migrate Swift cross-language bundle production to descriptor-bound
      signing and descriptor-ref ability names.
- [x] Migrate Swift README and runnable receipt-authority example away from
      plain signing.
- [x] Extend the V2 gate and self-test to reject Swift public plain helper
      definitions and public example usage.
- [x] Verify Swift focused tests, V2 gate/self-test, and public API manifest
      refresh before committing this slice.

## RF-3 Go Legacy Plain Fixture Naming Hardening Slice

- [x] Identify Go production invocation source that still used retired plain
      helper names after public export removal.
- [x] Rename Go plain helper logic to package-private `legacyPlain*` fixture
      functions.
- [x] Migrate Go axiom, vector, and admission tests to the explicit legacy
      fixture names.
- [x] Update comments and internal error naming so production source does not
      describe plain helpers as the canonical proof path.
- [x] Extend the V2 gate and self-test to reject retired Go plain helper names
      in non-test invocation source.
- [x] Verify Go invocation/full SDK tests, V2 gate/self-test, and public API
      manifest refresh before committing this slice.

## RF-3 Rust Legacy Plain Fixture Naming Hardening Slice

- [x] Identify Rust invocation source that still used retired plain helper
      names after public export removal.
- [x] Rename Rust plain helper logic to explicit `legacy_plain*` fixture
      functions.
- [x] Preserve descriptor-bound public proof/admission helpers and keep the
      shared signature-bytes verifier neutral.
- [x] Migrate Rust axiom, bundle, and admission tests to the explicit legacy
      fixture names.
- [x] Update comments and internal error naming so Rust production source does
      not describe plain helpers as the canonical proof path.
- [x] Extend the V2 gate and self-test to reject retired Rust plain helper
      names in invocation source.
- [x] Verify Rust focused/full SDK tests, V2 gate/self-test, and public API
      manifest refresh before committing this slice.

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

## RF-3 Node Public Plain Proof Helper Removal Slice

- [x] Identify Node public plain proof/admission helpers:
      `canonicalInvocationBytes`, `signInvocation`,
      `verifyInvocationSignature`, `verifySignature`, and `runAdmission`.
- [x] Remove those helper names from Node root and invocation public exports.
- [x] Rename retained historical plain vector helpers to explicit
      `legacyPlain*` internal fixture names.
- [x] Regenerate Node JS and declaration outputs so public type declarations
      expose descriptor-bound proof/admission only.
- [x] Migrate the Node cross-language bundle producer to descriptor-bound
      signatures and descriptor-ref ability names.
- [x] Extend the V2 gate and self-test to reject Node public plain helper
      reintroduction.
- [x] Verify Node axiom/admission/cross-language tests, Node axiom vector
      runner, Node build, and V2 gate.
- [x] Record that RF-3 remains open for remaining language/export/vector
      cleanup.

## RF-3 Java Public Plain Proof Helper Removal Slice

- [x] Identify Java public static plain proof/admission helpers:
      `canonicalInvocationBytes`, `signInvocation`,
      `verifyInvocationSignature`, `verifySignature`, and `runAdmission`.
- [x] Rename the Java plain helper group to package-private `legacyPlain*`
      fixture methods.
- [x] Preserve descriptor-bound Java proof/admission methods as the public
      runtime proof surface.
- [x] Migrate Java same-package vector/admission tests to the explicit legacy
      fixture names.
- [x] Migrate the Java cross-language bundle producer to descriptor-bound
      signatures and descriptor-ref ability names.
- [x] Extend the V2 gate and self-test to reject Java public static plain
      helper reintroduction.
- [x] Verify Java axiom/admission/cross-language tests and V2 gate/self-test.
- [x] Record that RF-3 remains open for Swift and remaining package/export
      cleanup.

## RF-3 Python Legacy Plain Fixture Naming and Producer Hardening Slice

- [x] Identify Python private plain proof/admission helpers that still used
      retired canonical names after public export removal.
- [x] Rename Python private plain helper logic to explicit
      `_legacy_plain*` fixture functions without keeping old aliases.
- [x] Preserve descriptor-bound Python proof/admission helpers as the public
      runtime proof surface.
- [x] Migrate Python axiom vector tests to the explicit legacy fixture names.
- [x] Migrate the Python cross-language bundle producer to descriptor-bound
      signatures, descriptor-ref ability names, and descriptor-derived proof
      facts.
- [x] Extend the V2 gate and self-test to reject retired Python private plain
      helper names in SDK source.
- [x] Verify Python focused tests, V2 gate/self-test, and public API manifest
      refresh before committing this slice.

## RF-3 Node Production Legacy Plain Export Removal Slice

- [x] Identify that Node `legacyPlain*` proof/admission helpers still lived in
      production invocation modules after public plain helper removal.
- [x] Delete Node production `legacyPlainInvocationBytes`,
      `signLegacyPlainInvocation`,
      `verifyLegacyPlainInvocationSignature`, `verifyLegacyPlainSignature`,
      and `runLegacyPlainAdmission`.
- [x] Move historical plain vector coverage to an explicit
      `scripts/legacy-plain-fixtures.mjs` fixture outside production SDK
      source.
- [x] Migrate Node admission tests to descriptor-bound signing and
      `runDescriptorBoundAdmission`.
- [x] Extend the V2 gate and self-test to reject Node production reintroduction
      of legacy plain proof/admission names.
- [x] Verify Node focused tests, package verify, V2 gate/self-test, and public
      API manifest refresh before committing this slice.

## RF-3 Go Production Legacy Plain Implementation Removal Slice

- [x] Identify that Go `legacyPlain*` proof/admission helpers still lived in
      production invocation modules after public helper removal and fixture
      renaming.
- [x] Delete Go production `legacyPlainInvocationBytes`,
      `signLegacyPlainInvocation`,
      `verifyLegacyPlainInvocationSignature`, `verifyLegacyPlainSignature`,
      and `runLegacyPlainAdmission`.
- [x] Move historical plain vector coverage to
      `legacy_plain_fixtures_test.go` so it is compiled only for tests.
- [x] Migrate Go module-level admission tests to descriptor-bound signing and
      `RunDescriptorBoundAdmission`.
- [x] Extend the V2 gate and self-test to reject Go production
      reintroduction of legacy plain proof/admission names.
- [x] Verify Go focused/all package tests, V2 gate/self-test, and public API
      manifest refresh before committing this slice.

## RF-9 Protocol-Pack URA Vector Naming Slice

- [x] Identify `easynet-uri-v1.json` as an active protocol-pack conformance
      vector, not historical documentation.
- [x] Rename the vector artifact to `easynet-ura-v1.json`.
- [x] Rename vector schema fields from `input_uri` / `canonical_uri` to
      `input_ura` / `canonical_ura`.
- [x] Update the protocol-pack release plan to reference the URA-named vector.
- [x] Extend the V2 gate and self-test to reject URI-named protocol-pack URA
      vectors and URI field names.
- [x] Verify protocol-pack build, protocol-pack consumer inventory, V2
      gate/self-test, and CLI gate tests before committing this slice.

## RF-9 Active Invocation Normative Document URA Naming Slice

- [x] Identify `AXIOM.tex` and RFC-001 as active normative invocation
      documents whose identity vocabulary must match the `ura` proto fields.
- [x] Replace `URI + profile`, `string uri`, `caller.uri`, and related
      identity/proof wording with URA terminology.
- [x] Preserve architecture semantics, proto field numbers, canonical byte
      ordering, and resolver responsibilities.
- [x] Extend the V2 gate and self-test to reject URI identity vocabulary in
      those active normative documents.
- [x] Verify focused document scan, V2 gate/self-test, and CLI convergence
      checks before committing this slice.

## RF-9 Keyring Resolver URA Naming Slice

- [x] Identify RFC-002 as an active key-custody and KeyResolver contract.
- [x] Replace `string uri` with `string ura` in the AgentIdentity key-field
      example.
- [x] Replace `peer_uri` with `peer_ura` in the keyring projection and ability
      surface examples.
- [x] Replace `find_peer_by_uri` with `find_peer_by_ura` in PeerKeyringResolver
      pseudocode.
- [x] Extend the V2 normative-document gate and self-test to cover RFC-002
      keyring/keyresolver URI terminology.
- [x] Verify focused document scan, V2 gate/self-test, and CLI convergence
      checks before committing this slice.

## RF-1 React Tool Adapter Product Surface Removal Slice

- [x] Identify React `tool_adapter` as an uncovered public SDK RF-1 surface.
- [x] Delete React `tool_adapter` TypeScript, JavaScript, and declaration
      source files instead of retaining a compatibility hook.
- [x] Remove `useAbilityTools` from React root exports, export tests, README,
      and SDK skill guidance.
- [x] Make the React type build clear stale generated declarations before
      regenerating package types.
- [x] Extend the V2 Axon product-boundary gate and self-test to reject React
      tool-adapter artifacts and public source/docs.
- [x] Verify React package tests/types, V2 gate/self-test, and CLI convergence
      checks before committing this slice.

## RF-9 Active Proto URA Vocabulary Slice

- [x] Identify active Axon proto comments that still used URI terminology for
      canonical device URA enumeration.
- [x] Update the canonical `core/proto/axon/v1/federation.proto` source from
      device `URIs` to device `URAs`.
- [x] Regenerate the runtime client-sdk and Rust SDK proto mirrors through
      `scripts/proto/sync_axon_v1.sh --write`.
- [x] Extend the V2 gate and self-test to reject URI terminology in Axon
      active proto schema roots.
- [x] Verify proto mirror derivation, V2 gate/self-test, and CLI convergence
      checks before committing this slice.

## Product-Neutral SDK URA Error Contract Slice

- [x] Identify SDK public validation errors that embedded `EasyNet URA`
      product wording in canonical runtime facades.
- [x] Replace Go, Java, Node, Python, Rust, and Swift subject/principal URA
      validation messages with product-neutral `canonical URA` or
      `URA syntax` wording.
- [x] Rename Swift local-runtime `SYSTEM_URI` to `SYSTEM_URA`.
- [x] Update language tests that asserted the old branded error text.
- [x] Extend the V2 gate and self-test to reject product-specific URA error
      vocabulary and active `SYSTEM_URI` identifiers in SDK source.
- [x] Verify focused language tests, V2 gate/self-test, and CLI convergence
      checks before committing this slice.

## RF-6 Cross-Language Receipt Anchor Fixture Convergence Slice

- [x] Identify Java full-suite failure as a shared receipt-anchor fixture fork,
      not a Java-only canonical encoder defect.
- [x] Promote the receipt authority anchor fixture from empty proof facts to a
      complete language-neutral proof-facts binding.
- [x] Use one runtime-env string,
      `axon-receipt-anchor-v2`, across Rust, Java, Python, Node, and Swift so
      signed receipt bytes do not fork by language.
- [x] Add the Rust SDK strict proof-facts anchor test as the executable source
      of truth for the new receipt anchors.
- [x] Migrate Java, Python, Node, and Swift anchor tests to the same
      subject-ref, descriptor version, schema hash, impl hash, input hash, and
      output hash fixture.
- [x] Remove Python's obsolete assertion that a missing receipt authority
      silently defaults to self authority; missing authority is now rejected.
- [x] Verify Rust, Java, Python, Node, and Swift affected suites.

## RF-6 Python Fluent Receipt Proof-Facts Boundary Slice

- [x] Identify Python `ReceiptSession(...).call(...)` as a public fluent path
      that generated signed receipts with `ReceiptProofFacts()` defaults.
- [x] Move proof-facts ownership to the call boundary:
      `.call(payload, proof_facts=...)` now requires explicit proof facts.
- [x] Remove the obsolete `prove_authority()` dummy `ReceiptBody` construction
      that preserved a no-proof receipt path inside the authority wrapper.
- [x] Update Python fluent tests, README guidance, and the authority receipt
      example to pass complete descriptor/runtime proof facts.
- [x] Add negative coverage proving fluent receipt construction rejects
      missing proof facts.
- [x] Regenerate the canonical public API manifest and SDK parity matrix after
      the Python public receipt API hash changed.
- [x] Verify Python authority, audit, admission, cross-language, and example
      paths.

## RF-6 Java Empty Receipt Proof Helper Removal Slice

- [x] Identify Java `Axiom.ReceiptProofFacts.empty()` as an obsolete public
      helper that preserved empty proof facts after Java receipt constructors
      and LocalRuntime production paths had moved to explicit facts.
- [x] Delete the public empty proof-facts helper instead of retaining a
      compatibility wrapper.
- [x] Migrate the Java receipt-closure example to build explicit
      descriptor/runtime proof facts at the receipt signing boundary.
- [x] Migrate Java receipt verb tests to pass explicit proof facts for signed,
      hosted, and causal-parent receipt forms.
- [x] Bind scalar/list causal parents into the explicit Java proof facts so
      example/test receipts no longer drop causal evidence at the proof-fact
      boundary.
- [x] Regenerate the canonical public API manifest and SDK parity matrix after
      the Java public API hash changed.
- [x] Verify the Java receipt verb suite, Java full SDK test suite, example
      receipt authority flow, and V2/architecture gates.

## RF-6 Go Empty Receipt Proof Helper Removal Slice

- [x] Identify Go `EmptyReceiptProofFacts()` as an obsolete production SDK
      helper after LocalRuntime receipt emission moved to explicit proof facts.
- [x] Delete the helper instead of retaining an empty-proof compatibility
      constructor.
- [x] Add a Go test-owned receipt proof-facts fixture helper for ordinary
      receipt verb and signature tests.
- [x] Migrate Go receipt verb and signature roundtrip tests away from empty
      proof facts.
- [x] Migrate Go authority-anchor tests to the shared
      `axon-receipt-anchor-v2` strict proof-facts pins used by the other
      language anchor suites.
- [x] Extend the V2 gate and self-test to reject `EmptyReceiptProofFacts()`
      anywhere in the Go invocation package.
- [x] Verify Go invocation tests, full Go SDK tests, V2 gate/self-test, and
      public API manifest refresh before committing this slice.

## RF-6 Swift Empty Receipt Proof Helper Removal Slice

- [x] Identify Swift `ReceiptProofFacts.empty` and defaulted
      `ReceiptProofFacts(...)` parameters as obsolete receipt-construction
      surfaces after LocalRuntime receipt emission moved to explicit facts.
- [x] Delete the Swift empty receipt proof-facts helper and unchecked
      receipt-facts initializer instead of preserving a compatibility path.
- [x] Require explicit arguments for Swift `ReceiptProofFacts` construction.
- [x] Add Swift `LocalReceiptProofFacts` ownership for descriptor-bound and
      system-local LocalRuntime receipt facts.
- [x] Refresh Swift proof-fact output hashes through
      `AxiomBinding.withPayloadDigest` when receipts are emitted.
- [x] Migrate Swift receipt-authority example, authority-method tests,
      bundle tests, cross-language verifier tests, and signed invocation tests
      away from empty proof facts.
- [x] Remove fixture-level authority fallback in Swift authority-method tests;
      callers pass the intended authority binding explicitly.
- [x] Extend the V2 gate and self-test to reject Swift empty receipt proof
      helpers, empty constructor calls, and receipt authority fallback shapes.
- [x] Verify Swift focused tests and V2 gate/self-test before committing this
      slice.

## RF-6 Node Empty Receipt Proof Helper Removal Slice

- [x] Identify Node `EMPTY_RECEIPT_PROOF_FACTS` as an obsolete public SDK
      helper after LocalRuntime receipt emission moved to explicit proof facts.
- [x] Delete the Node empty receipt proof-facts helper from the invocation
      source and root package exports.
- [x] Migrate the Node cross-language verifier fixture to construct explicit
      normalized authority proof and causal parent receipt facts.
- [x] Delete the excluded TypeScript authority-anchor test that preserved old
      empty-proof receipt anchors beside the active shared anchor suite.
- [x] Rebuild/check the Node SDK so generated JS/declaration artifacts no
      longer expose the empty helper.
- [x] Extend the V2 gate and self-test to reject `EMPTY_RECEIPT_PROOF_FACTS`
      anywhere under Node SDK source or tests.
- [x] Regenerate the canonical public API manifest after the Node public API
      hash changed.
- [x] Verify Node receipt-anchor tests, cross-language verifier tests, full
      Node SDK verification, and V2 gate/self-test before committing this
      slice.

## RF-6 Python Receipt Proof Constructor Hardening Slice

- [x] Identify Python `ReceiptProofFacts` dataclass defaults as an omitted
      proof-facts constructor path after LocalRuntime receipt emission moved
      to explicit facts.
- [x] Remove default values from Python `ReceiptProofFacts` fields so callers
      must provide subject, descriptor, runtime, authority, input/output, and
      causal parent facts explicitly.
- [x] Migrate Python audit, fluent authority, cross-language verifier,
      projection, authority-anchor, and example receipt fixtures away from
      empty `ReceiptProofFacts()` construction.
- [x] Preserve existing shared authority-anchor values as explicit fixture
      facts rather than changing cross-language anchors in a Python-only
      slice.
- [x] Extend the V2 gate and self-test with a Python AST scan that rejects
      empty `ReceiptProofFacts()` calls across SDK source, tests, and
      examples.
- [x] Verify focused Python receipt tests, V2 gate/self-test, and public API
      manifest refresh before committing this slice.

## RF-6 Rust Receipt Proof Default Constructor Removal Slice

- [x] Identify Rust `ReceiptProofFacts: Default`,
      `ReceiptProofFacts::default()`, and `proof_facts: Default::default()`
      as omitted receipt proof-facts construction paths.
- [x] Remove the `Default` implementation from Rust `ReceiptProofFacts`.
- [x] Add an explicit `ReceiptProofFacts::new(...)` constructor for the full
      subject, descriptor, runtime, authority, input/output, and causal-parent
      fact tuple.
- [x] Migrate `InvocationCore::new_with_policy` away from empty facts by
      deriving complete local proof facts from the provided `AxiomBinding`.
- [x] Refactor LocalRuntime receipt proof normalization so runtime-owned
      omitted facts are constructed directly from the admitted envelope and
      registered descriptor proof binding, while supplied facts must already
      be complete and matching.
- [x] Migrate Rust tests away from default receipt proof facts and add
      negative tests for supplied facts missing descriptor version or subject
      ref.
- [x] Extend the V2 gate and self-test to reject Rust receipt proof-facts
      default calls and `Default` derive reintroduction.
- [x] Verify focused Rust tests, Rust SDK check, V2 gate/self-test, and public
      API manifest refresh before committing this slice.

## RF-6 Python Authority Proof Constructor Hardening Slice

- [x] Identify Python `InvocationAuthorityProof` dataclass defaults as an
      omitted authority-proof constructor path nested inside receipt proof
      facts.
- [x] Remove default values from Python `InvocationAuthorityProof` fields so
      callers must provide proof type, binding, payload, hash, issuer,
      signature, and admission hook explicitly.
- [x] Migrate Python audit, fluent authority, cross-language verifier,
      projection, authority-anchor, and example receipt fixtures to explicit
      authority-proof construction.
- [x] Preserve existing shared authority-anchor values as explicit empty
      authority-proof fixture facts rather than changing cross-language
      anchors in a Python-only slice.
- [x] Extend the V2 gate and self-test with a Python AST scan that rejects
      authority-proof dataclass defaults and incomplete
      `InvocationAuthorityProof(...)` calls.
- [x] Verify focused Python receipt tests, V2 gate/self-test, and public API
      manifest refresh before committing this slice.

## RF-6 Node Empty Authority Proof Helper Removal Slice

- [x] Identify Node `EMPTY_AUTHORITY_PROOF` as an obsolete public SDK helper
      after receipt proof facts moved to explicit construction.
- [x] Delete the Node empty authority-proof helper from invocation source and
      root package exports.
- [x] Migrate the Node receipt-authority anchor suite to a file-local explicit
      authority-proof fixture instead of importing a public empty helper.
- [x] Rebuild/check the Node SDK so generated JS/declaration artifacts no
      longer expose the empty helper.
- [x] Extend the V2 gate and self-test to reject `EMPTY_AUTHORITY_PROOF`
      anywhere under Node SDK source or tests.
- [x] Verify Node receipt-anchor tests, cross-language verifier tests, full
      Node SDK verification, and V2 gate/self-test before committing this
      slice.

## RF-6 Java Empty Authority Proof Helper Removal Slice

- [x] Identify Java `InvocationAuthorityProof.empty()` as an obsolete public
      SDK helper after receipt proof facts moved to explicit construction.
- [x] Delete the Java empty authority-proof factory from `Axiom`.
- [x] Migrate the Java receipt-closure example to a file-local explicit
      authority-proof fixture instead of calling a public empty helper.
- [x] Migrate Java receipt authority, receipt verb, and cross-language
      verifier tests to a package-private explicit fixture.
- [x] Extend the V2 gate and self-test to reject
      `InvocationAuthorityProof.empty()` anywhere under Java SDK source,
      examples, or tests.
- [x] Verify focused Java receipt tests and V2 gate/self-test before
      committing this slice.

## RF-6 Swift Authority Proof Constructor Hardening Slice

- [x] Identify Swift `InvocationAuthorityProof.empty` and defaulted
      `InvocationAuthorityProof(...)` parameters as omitted authority-proof
      construction paths nested inside explicit receipt proof facts.
- [x] Delete the Swift empty authority-proof singleton and unchecked private
      initializer.
- [x] Require explicit values for every Swift `InvocationAuthorityProof`
      field at construction time.
- [x] Migrate Swift LocalRuntime proof facts, receipt-authority example,
      authority-method tests, bundle tests, cross-language verifier tests,
      and authority-anchor tests to explicit authority-proof construction.
- [x] Preserve the shared empty authority anchor only as a test-local explicit
      fixture.
- [x] Extend the V2 gate and self-test to reject Swift empty authority-proof
      helpers, `.empty` receipt usage, empty construction, and defaulted
      authority-proof initializer parameters.
- [x] Verify focused Swift authority tests, V2 gate/self-test, and repository
      gates before committing this slice; record the unrelated full Swift
      suite `MessageInboxIdempotentTests` residual failure without claiming
      full-suite success.

## RF-6 Go Zero Authority Proof Fixture Removal Slice

- [x] Identify Go `InvocationAuthorityProof{}` literals in receipt authority
      anchor and cross-language verifier tests as omitted authority-proof
      fixtures nested inside explicit receipt proof facts.
- [x] Replace bare zero-value authority proof literals with a named
      test-local `anchorAuthorityProof()` fixture that spells out every
      authority-proof field.
- [x] Preserve current shared receipt authority anchor bytes while making the
      empty authority fixture explicit.
- [x] Extend the V2 gate and self-test to reject bare
      `InvocationAuthorityProof{}` in the Go invocation package, excluding
      JSON decode error returns in `bundle.go`.
- [x] Verify focused Go invocation tests and V2 gate/self-test before
      committing this slice.

## RF-6 Rust Authority Proof Default Removal Slice

- [x] Identify Rust `InvocationAuthorityProof: Default`,
      `InvocationAuthorityProof::default()`, authority-proof struct update
      defaults, and verifier `ReceiptProofFacts { ..Default::default() }`
      as remaining omitted authority/proof-fact construction paths.
- [x] Remove `Default` derive from Rust `InvocationAuthorityProof`.
- [x] Add explicit `InvocationAuthorityProof::new(...)` constructor requiring
      proof type, binding, payload, hash, issuer, signature, and admission
      hook inputs.
- [x] Migrate Rust audit, LocalRuntime proof normalization, wire tests,
      axiom tests, shared anchor fixtures, and verifier integration tests to
      explicit authority proof construction.
- [x] Replace verifier `ReceiptProofFacts { ..Default::default() }` with
      `ReceiptProofFacts::new(...)`.
- [x] Extend the V2 gate and self-test to reject Rust
      `InvocationAuthorityProof` default derive/calls and
      `ReceiptProofFacts { ..Default::default() }`.
- [x] Verify focused Rust authority, receipt normalization, and verifier
      tests plus V2 gate/self-test before committing this slice.

## RF-6/RF-3 Runtime Client Receipt Proof Adapter Hardening Slice

- [x] Identify `core/runtime-rs/client-sdk/src/domain/admission.rs` as a
      duplicate Rust proof adapter that still derived `ReceiptProofFacts:
      Default`, made `authority_proof` optional, and synthesized an empty
      canonical `InvocationAuthorityProof`.
- [x] Make runtime client `ReceiptProofFacts.authority_proof` required
      protobuf transport data.
- [x] Route protobuf authority-proof conversion through canonical
      `InvocationAuthorityProof::new(...)` instead of a default value.
- [x] Migrate runtime admission wire, receipt emitter, offline verifier, and
      runtime test helpers to required authority proof facts.
- [x] Fail closed when an admission receipt lacks the authority proof used to
      rebuild terminal receipt proof facts.
- [x] Extend the V2 gate/self-test to reject runtime client adapter
      `ReceiptProofFacts: Default`, `authority_proof: Option<...>`, and
      `InvocationAuthorityProof::default()`.
- [x] Verify client-sdk, runtime, verifier, V2 gate/self-test, and diff
      hygiene before committing this slice.

## RF-3 Rust Legacy Plain Proof Implementation Removal Slice

- [x] Identify Rust `legacy_plain_invocation_bytes`,
      `sign_legacy_plain_invocation`,
      `verify_legacy_plain_invocation_signature`,
      `verify_phase_legacy_plain`, and `run_legacy_plain_admission` as the
      remaining private legacy plain proof/admission implementation path.
- [x] Delete the private legacy plain encoder, signer, verifier, verify
      phase, and admission runner instead of keeping test-only compatibility
      wrappers.
- [x] Migrate axiom, admission, and bundle tests to
      `DescriptorBoundEnvelope` canonical bytes and descriptor-bound
      signature helpers.
- [x] Replace invalid arbitrary subject/callee fixture URAs with valid
      runtime URAs so descriptor-bound validation is exercised by tests.
- [x] Extend the V2 gate/self-test to reject Rust legacy plain
      proof/admission helper names in invocation source.
- [x] Verify Rust invocation tests, Rust SDK check, V2 gate/self-test, CLI
      conformance tests, and diff hygiene before committing this slice.

## RF-3 Java Legacy Plain Proof Implementation Removal Slice

- [x] Identify Java package-private `legacyPlainInvocationBytes`,
      `signLegacyPlainInvocation`,
      `verifyLegacyPlainInvocationSignature`, and
      `runLegacyPlainAdmission` as a production legacy plain
      proof/admission implementation path.
- [x] Delete the Java production legacy plain encoder, signer, verifier, and
      admission runner instead of preserving package-private compatibility
      fixtures.
- [x] Migrate Java admission tests to `runDescriptorBoundAdmission` and
      descriptor-bound signatures.
- [x] Migrate Java axiom-vector tests to descriptor-bound canonical bytes,
      signing, and verification.
- [x] Extend the V2 gate/self-test to reject Java legacy plain
      proof/admission helper names in production invocation source.
- [x] Verify focused Java invocation tests, V2 gate/self-test, CLI
      conformance tests, and diff hygiene before committing this slice.

## RF-3 Python Legacy Plain Proof Implementation Removal Slice

- [x] Identify Python `_legacy_plain_invocation_bytes`,
      `_sign_legacy_plain_invocation`,
      `_verify_legacy_plain_invocation_signature`,
      `_verify_legacy_plain_signature`, and
      `_run_legacy_plain_admission` as a production legacy plain
      proof/admission implementation path.
- [x] Delete the Python production legacy plain encoder, signer, verifier,
      admission verifier, and admission runner instead of preserving private
      compatibility fixtures.
- [x] Migrate Python axiom-vector tests to descriptor-bound canonical bytes,
      signing, and verification.
- [x] Extend the V2 gate/self-test to reject Python legacy plain
      proof/admission helper names across Python SDK source, tests, and
      examples.
- [x] Verify focused Python invocation tests, V2 gate/self-test, CLI
      conformance tests, and diff hygiene before committing this slice.
- [x] Record that full `sdk/python/tests` still has unrelated industrial
      lifecycle failures around `core`, `children_of`, `send_message`, and
      `cancel`; those remain RF-4 lifecycle facade debt.

## Still Required Before Completion

- [ ] RF-5 cross-language signer-handle and daemon KeyService convergence.
- [ ] RF-3 remaining language package/vector/example audit for
      descriptor-bound-only public proof.
- [ ] RF-8/RF-7 complete tuple ingress and LocalRuntime-only daemon routes.
- [ ] RF-4 shared lifecycle matrix, transition vectors, and cross-language
      provider status cutover.
- [ ] RF-6 final cross-language constructor hardening, remaining
      package/example audit, and descriptor proof-binding parity closure.
- [ ] RF-1 product SDK surface extraction.
- [ ] RF-2 Mission/EAL extraction from Axon canonical runtime.
- [ ] RF-9 URA terminology and generated-schema ownership closure.
