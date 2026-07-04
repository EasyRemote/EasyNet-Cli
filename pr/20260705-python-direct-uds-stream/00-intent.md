# Python direct UDS stream transport

## Goal

Implement the Python SDK direct daemon `InvokeStream` adapter over Axon
gRPC-over-UDS without exposing generated protocol modules to product callers.

The direct transport remains a facade over daemon/Axon truth:

- Axon owns stream wire shape, terminal states, receipts, and ordering rules.
- EasyNet-Cli daemon owns stream dispatch policy and endpoint advertisement.
- Python SDK owns connection lifetime, DTO projection, bounded local buffering,
  and SDK `StreamHandle` compatibility.

## Non-goals

- Do not implement direct bidi in this slice.
- Do not implement direct prepare/submit in this slice.
- Do not alter the daemon SDK requirements spec.
- Do not invent stream terminal semantics beyond Axon chunk projection and the
  existing SDK `StreamHandle` state machine.
