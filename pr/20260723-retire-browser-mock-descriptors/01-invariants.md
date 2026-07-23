# Invariants

## Semantic

- A descriptor in the active system inventory is a public runtime contract.
- A `cutover_ready` descriptor requires a real executable handler path.
- Descriptor-only placeholder abilities are not seams; they are false runtime
  contracts.

## Safety

- Removing the descriptors is safer than advertising an ability whose route
  cannot terminate in a signed receipt.
- Browser functionality must re-enter through a fresh implementation with a
  real lifecycle state machine and executable LocalRuntime route.

## Boundedness

- No caller should wait on a non-existent route for a descriptor-only browser
  ability.
- No fallback route may synthesize mock browser receipts.

## Recovery

- The removal leaves no session state, receipt state, or compatibility shim.
