Decision log:

- Treat route owner liveness as routing availability, not descriptor absence.
- Preserve compatibility with already-released daemon payloads by canonicalizing
  legacy `NOT_FOUND + ROUTE_NEGATIVE` details in SDK direct providers.
