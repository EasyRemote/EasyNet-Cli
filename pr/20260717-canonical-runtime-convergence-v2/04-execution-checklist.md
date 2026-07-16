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

## Still Required Before Completion

- [ ] RF-5/RF-3 signer custody and descriptor-bound proof cutover.
- [ ] RF-8/RF-7 complete tuple ingress and LocalRuntime-only daemon routes.
- [ ] RF-4 shared lifecycle matrix and transition vectors.
- [ ] RF-6 receipt proof-fact constructor closure.
- [ ] RF-1 product SDK surface extraction.
- [ ] RF-2 Mission/EAL extraction from Axon canonical runtime.
- [ ] RF-9 URA terminology and generated-schema ownership closure.
