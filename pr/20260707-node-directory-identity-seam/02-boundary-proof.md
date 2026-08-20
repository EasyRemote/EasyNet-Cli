# Boundary Proof

Correct boundary:

```text
Node caller
  -> Node DirectoryClient / IdentityClient seam
  -> injected product/provider transport
  -> daemon/Axon-owned projection
```

Incorrect boundary:

```text
Node caller
  -> Node-local URA/DescriptorRef parser or directory fan-out loop
```

The seam only serializes canonical SDK request DTOs, validates bounded request
shape, and projects JSON objects returned by the transport. It deliberately
does not know Axon grammar or daemon routing policy.
