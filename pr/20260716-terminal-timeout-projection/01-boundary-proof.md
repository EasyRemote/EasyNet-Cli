# Boundary Proof

Axon owns canonical terminal states and receipt proof. The daemon `TerminalState`
is a presentation projection for schedule and kernel receipts, so it must be
one-to-one for every terminal Axon state:

- `Completed` -> `Succeeded`
- `Failed` -> `Failed { reason }`
- `TimedOut` -> `TimedOut { reason }`
- `Cancelled` -> `Cancelled`

`KernelLoopInvocationDriver` consumes that projection and must retain timeout
as a distinct terminal outcome. It may surface an operation error, but cannot
rewrite timeout as a handler failure.
