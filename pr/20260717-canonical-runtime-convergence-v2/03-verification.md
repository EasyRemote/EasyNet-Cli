# Canonical Runtime Convergence V2 - Verification

Before any root-fork slice is declared complete:

1. Run its repository-local unit, integration, and conformance suites.
2. Run all language lifecycle vectors for unary, stream, bidi, child deadline,
   cancellation, idempotent replay, terminal cleanup, and restart recovery.
3. Prove that no public path invokes plain admission or constructs a receipt
   with omitted proof facts.
4. Prove that each daemon ability route enters `LocalRuntime` through a
   descriptor-bound request, including stream and bidi routes.
5. Run URA terminology and product-neutral SDK surface gates across both
   repositories.
6. Compare fixed-baseline latency, allocation, cancellation cleanup time, and
   peak concurrent task count; report measurements rather than estimates.

## Documentation-Slice Results (2026-07-17)

- `git diff --check`: passed.
- `tools/scripts/check-architecture-convergence.sh`: passed.
- `tools/scripts/check-project-structure-v1.sh`: passed.
- The new normative SPEC and its plan pack contain no retired alternate
  address-term token.

No runtime benchmark was run because this slice changes only normative
documentation and planning records.
