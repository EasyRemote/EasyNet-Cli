# Descriptor Service Owner Vocabulary

## Problem

The canonical runtime convergence gate still expects descriptor validation
errors to describe `Agent, Device, or Authority` owners. Current descriptor
semantics correctly allow `Agent`, principal-projected `Service`, and realm
`Authority` owners; `Device` remains an execution substrate, not a descriptor
owner.

## Invariants

- AbilityDescriptor owner/callee identities are Agent, Service, or Authority.
- Device stays sponsor/execution host, not public descriptor owner.
- The gate must verify the implemented canonical owner vocabulary rather than
  pushing the old Device-owner wording back into production code.

## Implementation

- Update `check-canonical-runtime-convergence-v2.sh` to require
  `canonical Agent, Service, or Authority URA` in descriptor validation errors.

## Verification

- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
