# Decisions

## Handle Snapshot Objects

`RuntimeClient.submitSigned` now returns `InvocationHandle` instead of an
anonymous object. The handle is a submitted-observation DTO with methods that
delegate back to its bound `RuntimeClient`.

## Honest Conformance Boundary

This slice intentionally does not add Node to
`invocation/handle_terminal_monotonicity`. The implementation covers submitted
handle observation, but the shared case also requires prepare/sign actions. That
claim belongs with the prepare/sign Runtime Core slice.
