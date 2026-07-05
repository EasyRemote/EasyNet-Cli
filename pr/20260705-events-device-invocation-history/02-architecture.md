# Architecture

## Layering

1. Rust `daemon::events_contract` validates event requests, builds complete Invocation carriers, and projects daemon event history pages.
2. `src/ffi/events` exports those contract functions through C ABI v4.
3. Python `_cabi.CABIEventTransport` calls the C ABI carrier builders and executes Runtime Core stream/invoke operations.
4. Python `EventClient` keeps the existing OOP facade and typed DTOs.

## Boundary Proof

- The SDK does not create or store events. It delegates to daemon governed abilities/read models.
- Runtime Core owns stream handle lifecycle, cancellation, and terminal observation.
- Backend/GUI layers may fan out projected events, but do not own daemon subscription carriers.
