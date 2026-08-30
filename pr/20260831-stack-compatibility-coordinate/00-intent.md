# Intent

## Goal

Make one committed CLI lock the exact Axon compatibility coordinate used by local verification, pull-request CI, clippy, and Runtime artifact builds.

## Non-goals

- Do not declare the current Axon candidate compatible while CLI admission tests fail.
- Do not replace registry-only artifact verification with sibling path checks.
- Do not change public Runtime or SDK behavior merely to make a version gate green.
- Do not preserve duplicated workflow-owned revision facts.

## Acceptance criteria

- Current CLI baseline failures are repaired or attributed to an exact incompatible Axon candidate.
- `compatibility/axon.lock.json` binds one Axon revision, release version, contract digest, protocol digest, ABI version, and SDK versions.
- One checker validates the Axon checkout, its contract, CLI Rust/Python resolution, and lock drift.
- All workflows resolve their checkout revision from the lock; none hard-code `EASYNET_AXON_COMPAT_REV`.
- Pinned admission is mandatory, candidate admission is explicit, and artifact checks reject development path sources.
