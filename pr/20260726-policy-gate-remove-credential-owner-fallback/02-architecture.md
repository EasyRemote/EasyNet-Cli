# Architecture

## Before

`AdmissionPolicyGate` resolved device owner facts by consulting the trust anchor and then falling back to `local_device_owner_fact`, which reads saved pairing credentials. That made ordinary policy admission depend on a local product state file.

## After

`AdmissionPolicyGate` resolves device ownership only from trust-anchor owner rows. Local credential projection remains in `BootstrapAuthority`, where it is bounded to first-publication control-plane abilities.

## Boundary

- Core policy gate: trust/URA/verified authority only.
- Bootstrap verifier: temporary pairing credential evidence under explicit ability/action constraints.
- Persistence config: not consulted by ordinary policy admission.
