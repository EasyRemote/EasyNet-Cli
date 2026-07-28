# Runtime readiness product error convergence

Date: 2026-07-29

## Goal

Close the product-visible error gap where invocation history, catalogue reads,
and descriptor resolution can surface low-level route/keyring failures instead
of the canonical runtime readiness and provider-boundary states.

## Invariants

1. `invocation.history.*` is not a public action route. It must enter through
   the canonical receipt-history read provider.
2. `meta.list_abilities` and `meta.list_resources` catalogue reads are not
   public action routes. They must enter through the canonical ability
   descriptor provider.
3. Runtime-state reads require a concrete paired User subject and live caller
   signer custody before publishing a runtime projection.
4. Clean local state must not silently synthesize compatibility credentials.
   A device runtime without credentials fails before daemon start; hub runtime
   bootstrap requires explicit TLS configuration.
5. Product-facing error projection must not leak keyring implementation details,
   route resolver internals, or stale descriptor lookup failures for these
   provider-bound routes.

## Work plan

1. Reproduce clean-state startup behavior after purging local data. Done:
   clean state refuses device start without credentials and refuses hub start
   without explicit TLS config.
2. Use codegraph and source search to locate active producers of retired
   invocation-history session subjects and generic governance public routes.
   Done: active Rust/Go/Python paths were already provider-bound; Node, Java,
   and Swift missed `meta.list_resources` in their catalogue governance-read
   public-ingress guard.
3. Strengthen the gate around provider-bound product reads so downsteam SDK/UI
   integrations cannot regress to generic public-route dispatch. Done:
   SPEC v2 gate now requires `meta.list_resources` across Node, Java, and Swift.
4. Run targeted SDK/runtime tests and SPEC v2 gate. Done:
   Node, Java, Swift, rustfmt, SPEC v2, SPEC v2 self-test, and legacy
   architecture convergence gate pass.
5. Commit only stable, scoped changes authored by Silan.Hu. Pending.

## Boundary decisions

- Do not add compatibility for deleted local state. The clean state requires
  explicit pairing or explicit hub bootstrap.
- Do not relax descriptor resolution to hide offline owners. Provider-bound
  routes must fail before descriptor/route lookup when the caller used the wrong
  ingress.
- Do not add product-specific SDK concepts. The SDK remains a canonical runtime
  model with typed providers.

## Result

- `meta.list_resources` is now a provider-bound runtime catalogue read in Node,
  Java, and Swift SDK public-ingress validation.
- Direct public invocation builders now reject `meta.list_resources` descriptor
  refs before descriptor resolution or runtime dispatch.
- Runtime ability public-build paths now reject `meta.list_resources` before
  transport, matching Go/Python behavior.
