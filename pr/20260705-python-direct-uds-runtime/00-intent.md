# Python direct UDS runtime transport

## Goal

Implement the Python SDK Runtime Core unary transport as a direct daemon
Invocation gRPC-over-UDS facade.

The SDK must remain a wrapper over daemon/Axon truth:

- Axon protocol model stays in EasyNet-Axon.
- EasyNet-Cli daemon owns dispatch semantics.
- Python SDK owns connection, DTO projection, and product-safe facade shape.
- Product code must not import generated proto modules or raw daemon internals.

## Non-goals

- Do not rewrite the spec.
- Do not reimplement URA, DescriptorRef, admission, scheduling, signing, or
  receipt verification semantics in Python.
- Do not replace stream/bidi/prepare/submit in this slice.
- Do not preserve obsolete SDK transport branches once a cleaner concrete path
  exists, except where public API compatibility requires an adapter.
