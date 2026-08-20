Boundary proof:

- Route resolution owns the first typed distinction between absence and
  liveness. It must not encode owner-offline as NotFound.
- SDK direct runtime owns gRPC status projection for providers. It must not
  infer descriptor absence from transport status alone when the daemon gives a
  route-negative owner-offline detail.
- Product code should only need the stable SDK code, not string matching on
  `ROUTE_NEGATIVE` or `owner is not online`.
