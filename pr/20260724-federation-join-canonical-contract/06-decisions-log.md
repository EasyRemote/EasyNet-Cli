# Decisions Log

## 2026-07-24

- Selected `federation.join` `pairing_secret` as this iteration's cut because it is a product/token-pairing compatibility carrier exposed in a canonical runtime capability schema while not contributing to descriptor-bound admission or receipt facts.
- Hub-side join parsing now rejects unknown fields with `deny_unknown_fields`; deleting the client/schema field alone would leave a hidden compatibility ingress where retired token carriers are silently ignored.
