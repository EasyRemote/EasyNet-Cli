# Pairing response strict schema cutover

## Goal

Make the backend pairing validate response an explicit product contract:
unknown fields and retired aliases must fail before credentials projection, and
tests must not construct incomplete pairing envelopes through `Default`.

## Non-goals

- Do not remove the backend HTTP pairing facade in this slice; it is still a
  product join path until SPEC cutover authorizes deletion.
- Do not change the public `easynet join` CLI arguments.
- Do not alter the canonical runtime SDK model; pairing remains a downstream
  EasyNet product credential contract.

## Acceptance criteria

1. `PairingCredentialEnvelope` rejects unknown fields at serde ingress.
2. Retired `tenant_id` cannot be carried alongside canonical `realm`.
3. Pairing DTOs with required lifecycle facts do not derive `Default`.
4. Join tests construct complete pairing envelopes through a single fixture.
5. SPEC v2 gate locks this strict schema boundary.
