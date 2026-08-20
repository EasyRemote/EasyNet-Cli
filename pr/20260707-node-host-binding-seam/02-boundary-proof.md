# Node Host Binding Seam Boundary Proof

## Ownership

Host Binding is a daemon SDK profile for host-stream DTOs and codec/hash
semantics. Node owns only a language facade and a local generic codec/hash
transport.

## Call Path

```text
Node product host
  -> HostBindingClient
  -> LocalHostBindingTransport or injected HostBindingTransport
  -> shared host-stream DTO and hash contract
```

The product host still owns sockets, process lifecycle, function execution, and
language-specific value binding.

## Rejected Designs

- Starting or supervising a product host process in SDK: rejected as product
  lifecycle.
- Parsing descriptor refs locally: rejected because identity/Axon helpers own
  canonical descriptor projection.
- Treating host-stream frames as product-specific messages: rejected because the
  shared schema is the SDK contract.
