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

## Still Required Before Completion

- [ ] RF-5/RF-3 signer custody and descriptor-bound proof cutover.
- [ ] RF-8/RF-7 complete tuple ingress and LocalRuntime-only daemon routes.
- [ ] RF-4 shared lifecycle matrix and transition vectors.
- [ ] RF-6 receipt proof-fact constructor closure.
- [ ] RF-1 product SDK surface extraction.
- [ ] RF-2 Mission/EAL extraction from Axon canonical runtime.
- [ ] RF-9 URA terminology and generated-schema ownership closure.
