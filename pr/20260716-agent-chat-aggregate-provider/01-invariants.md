# Invariants

- Invocation-facing Agent helpers consume one coherent Agent aggregate snapshot per provider call.
- The aggregate repository remains the only owner that pairs durable Agent registry and hosted identity projection reads.
- Hot-added discover and invoke handlers must observe newly registered peers through the same aggregate-owned registered-Agent projection.
- Handler provider errors must preserve their typed operational context instead of being hidden behind default registry values.
- Chat context peer-skill hints are advisory only; failure to load the aggregate must not block the chat turn.
