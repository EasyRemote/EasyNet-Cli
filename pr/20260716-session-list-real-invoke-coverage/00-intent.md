# Session List Real Invocation Coverage

## Concrete use case

`session.list` is a published device RPC. The real-invocation gate must prove
that its handler is called rather than report a false missing-coverage result
because the test uses a generated canonical route constant.

## Boundary proof

The generated runtime-admin route constant remains the canonical definition.
The real-invocation test declares the public route literal and asserts it is
equal to that constant before invoking the handler. A route rename now fails
the test instead of silently being counted by a heuristic.
