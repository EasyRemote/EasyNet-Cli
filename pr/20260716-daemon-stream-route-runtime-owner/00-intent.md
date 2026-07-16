# Intent

Converge daemon exact server-stream routes onto the shared LocalRuntime route
owner.

Exact unary routes already register into `LocalRuntime` and dispatch through
the descriptor-bound runtime adapter. Exact stream routes still bypass that
owner: `Invocation::invoke_stream` directly calls product stream handlers for
`federation.subscribe_directory` and `federation.subscribe_directory_v2`.

## Expected effect

- Architecture convergence: exact daemon routes have one runtime owner across
  unary and stream geometries.
- Receipt/proof clarity: stream exact routes cannot mint lifecycle frames from
  tonic/product code while unary routes use Axon finalization.
- Public behavior remains the same for directory subscribers.
