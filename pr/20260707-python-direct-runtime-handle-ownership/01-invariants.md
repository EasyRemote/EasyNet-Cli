# Invariants

- The direct daemon gRPC transport must not implement prepare, submit, await, cancel, event, or free-handle semantics itself.
- Handle transport delegation is explicit and optional.
- Handle transport ownership is explicit and defaults to non-owning.
- A connector-created direct transport must not close the connector's shared handle transport.
- A connector configured to own its handle transport closes that handle transport once, after closing direct transports.
- A standalone direct transport configured to own its handle transport closes that handle transport once, after closing its direct channel.
- Close must be idempotent.
- Close must attempt every owned resource and then surface the first close failure.
- No legacy aliases are introduced.
- Runtime naming remains generic and product-neutral.

