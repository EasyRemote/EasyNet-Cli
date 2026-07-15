# Daemon Ability Route Convergence Invariants

## Objective

Converge daemon ability invocation onto one live catalog source for descriptor
proofs, route publication snapshots, resolver answers, and runtime route
registration.

## Invariants

1. Local daemon route registration must read descriptor records from the
   daemon's live `local_ability_catalog`; it must not rebuild a parallel system
   registry.
2. Descriptor proof lookup is owner-aware. Hub, device, and hosted-agent rows
   for the same public ability name are distinct authority bindings.
3. System resource abilities such as `fs.*`, `context.fs.list`, and `skill.*`
   remain daemon/device system abilities. External provider file APIs such as
   `openai.files.*` remain integration abilities.
4. Runtime tests that bind custom local abilities must publish the same ability
   into the test local catalog used by resolver/admission.
5. No fallback route projection may exist only to preserve the old direct-route
   architecture.
6. Exact daemon unary routes are installed and executed only through
   `DaemonRouteRuntimeAdapter`; tonic dispatch may classify ingress, but it must
   not call product handlers directly.

## Validation Gates

1. CodeGraph sync and targeted exploration for catalog/route dependencies.
2. Focused daemon invocation service tests for unary, stream, bidi, and
   canonical forward paths.
3. Ability catalog assembly tests for Hub/Device authority leakage.
4. Architecture boundary scripts after the local test suite is stable.
5. `tools/scripts/check-architecture-convergence.sh` rejects missing or
   bypassed `DaemonRouteRuntimeAdapter` ownership for exact daemon routes.
