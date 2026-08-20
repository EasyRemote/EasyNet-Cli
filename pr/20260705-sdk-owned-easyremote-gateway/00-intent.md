# SDK-owned EasyRemote gateway facade

## Objective

Move EasyRemote Server/Gateway hub configuration materialization, lifecycle
state, TLS file validation, endpoint projection, and certificate fingerprint
projection behind the EasyNet-Cli Python SDK Admin + Gateway profile.

## Boundary

- The daemon remains responsible for hub runtime behavior, public listener
  readiness, pairing, trust, sessions, routing, and receipts.
- The SDK owns the reusable gateway facade mechanics already required by
  EasyRemote: create-once hub config, start/stop state, and operator-facing
  readiness facts derived from daemon inputs.
- EasyRemote keeps its public `Server`/`Gateway` names and self-signed
  certificate provisioning ergonomics.

## Non-goals

- Do not implement ACME.
- Do not fabricate pairing, credential, device-session, or hub membership
  lifecycle semantics before daemon/ABI contracts exist.
- Do not edit `docs/spec/daemon-sdk-requirements-v1.md`.
