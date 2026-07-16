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

## RF-5 Public Surface Signer Fallback Quarantine Slice

- [x] Identify `generate_subject_auth` and
      `runtime_admin.generate_subject_auth` as Rust public-surface evidence
      still counted in the canonical SDK capability graph.
- [x] Extend public-surface policy to classify generated/default subject auth,
      process-local signer, and private-key authenticator symbols as RF-5
      non-canonical signer fallback defects.
- [x] Extend the V2 manifest gate to reject fallback signer helpers in
      canonical language/member graphs.
- [x] Regenerate the canonical public API manifest and SDK parity matrix with
      `generate_subject_auth` moved to legacy quarantine.
- [x] Verify the V2 convergence script, canonical public API script, focused
      Cargo script-check wrapper, formatting, and lib build.
- [x] Record that upstream implementation removal remains open; this slice
      fixes conformance ownership, not the full RF-5 cutover.

## Still Required Before Completion

- [ ] RF-5/RF-3 signer custody and descriptor-bound proof cutover.
- [ ] RF-8/RF-7 complete tuple ingress and LocalRuntime-only daemon routes.
- [ ] RF-4 shared lifecycle matrix and transition vectors.
- [ ] RF-6 receipt proof-fact constructor closure.
- [ ] RF-1 product SDK surface extraction.
- [ ] RF-2 Mission/EAL extraction from Axon canonical runtime.
- [ ] RF-9 URA terminology and generated-schema ownership closure.
