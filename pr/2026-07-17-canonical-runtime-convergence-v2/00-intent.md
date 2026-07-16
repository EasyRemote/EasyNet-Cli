# Canonical Runtime Convergence V2 Intent

## Goal

Implement `docs/spec/canonical-runtime-convergence-v2.md` as the clean target
for EasyNet-Cli work in this repository. The objective is architecture
convergence: public behavior remains compatible while canonical runtime
responsibility moves into descriptor-bound Axon runtime entry points and
EasyNet-Cli keeps only daemon/product policy.

## Non-Negotiable Constraints

- SDK concepts are generic runtime concepts, not EasyNet or EasyRemote product
  concepts.
- Every public invocation boundary preserves the seven fields:
  `caller`, `callee`, `ability`, `subject`, `nonce`, `causal_context`, `args`.
- The daemon may classify policy and locality, but it does not construct a
  second canonical proof/admission model.
- Internal daemon calls use an explicit system issuer and enter the same
  descriptor-bound admission path.
- URA is the only active routable identity/address vocabulary.
- Compatibility is allowed only as a versioned edge adapter. It must not keep a
  second canonical implementation alive.

## Delivery Scope

This repository can directly close CLI-owned portions of the SPEC:

- RF-7/RF-8: daemon route and tuple ingress convergence.
- CLI-facing RF-3/RF-5: descriptor-bound admission usage and no local signing
  fallback in daemon code.
- RF-4/RF-6/RF-9 gates where this repository owns conformance evidence,
  lifecycle matrix files, receipt checks, terminology scans, and generated
  schema verification hooks.
- RF-1/RF-2 only where product-owned surfaces or Mission/EAL state exist inside
  this repository. Axon upstream removals remain cross-repository follow-up
  unless vendored/copied here.

## Iteration Rule

Each implementation slice must update the proof files in this plan pack with:

- owner decision;
- explicit state machine when lifecycle exists;
- caller inventory;
- migration and deletion list;
- automated gate;
- regression evidence.
