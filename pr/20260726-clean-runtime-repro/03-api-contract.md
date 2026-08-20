# API Contract

- Public SDK entry points remain generic runtime APIs.
- Error projection must use canonical runtime error codes.
- URA is the only architecture term for runtime names.
- Product ability names such as `invocation.history.list` are payload-level
  route inputs, not SDK abstractions.
