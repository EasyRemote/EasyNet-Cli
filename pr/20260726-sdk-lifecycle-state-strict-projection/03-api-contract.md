# API Contract

## Accepted receipt state values

- `Unspecified`
- `Accepted`
- `Admitted`
- `Dispatched`
- `Running`
- `Completed`
- `Failed`
- `TimedOut`
- `Cancelled`

`Unspecified` is parseable as a lifecycle value but invalid for a runtime receipt summary.

## Rejected values

- lowercase aliases such as `completed`
- screaming-case aliases such as `COMPLETED`
- snake-case aliases such as `TIMED_OUT`
- whitespace-padded variants such as ` Completed `
- punctuation-normalized variants such as `timed-out`
- unknown states

## Public compatibility

Existing public methods and return values remain compatible:

- Go returns `InvocationLifecycleState` constants.
- Python returns `InvocationLifecycleState`.
- Node returns existing uppercase lifecycle projection strings.
- Java returns existing uppercase lifecycle projection strings.
- Swift returns existing uppercase lifecycle projection strings.

Compatibility is preserved at the API shape level, not by accepting retired carrier spellings.
