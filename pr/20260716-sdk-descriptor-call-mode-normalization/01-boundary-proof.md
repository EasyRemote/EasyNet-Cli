# Boundary Proof

- Owner: `RuntimeClient.resolve_descriptor_ref` owns normalization of the
  optional descriptor-selection request before it crosses the transport seam.
- State transition: `empty | whitespace -> rpc`; non-empty mode remains
  caller-selected and is forwarded unchanged.
- Invariant: Go and Python emit the same `call_mode: "rpc"` provider request
  for an omitted/blank mode.
- Non-owner: typed ability providers and transports neither mint a default nor
  recover from a blank mode.
