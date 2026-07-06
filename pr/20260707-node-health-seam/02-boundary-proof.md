# Node Health Seam Boundary Proof

## Ownership

The SDK owns the language-neutral Health DTOs and typed facade. Daemon runtime,
control endpoint discovery, product health routes, backend status pages, and
process supervision remain outside this Node seam.

## Call Path

```text
Node consumer
  -> HealthClient
  -> injected HealthTransport
  -> shared health / diagnostics JSON DTOs
```

## Rejected Designs

- Starting or discovering a daemon in `HealthClient`: rejected as lifecycle and
  transport-provider ownership.
- Mapping health onto feature discovery: rejected because API liveness and
  runtime readiness are separate typed values.
- Accepting alternate field aliases: rejected because the SPEC requires latest
  canonical SDK schema fields only.
