# Companion CLI Status Reconciliation

## Goal

Make `easynet plugin status` follow the desktop companion SPEC online/offline contract: online daemon-local control ability status is compared with local manager observation, and stale daemon plugin state never overrides the local status DTO.

## Boundary

- The CLI may select between daemon-local control output and local manager output.
- The DTO shape remains owned by `src/protocol/companion_contract.rs`.
- Warnings are operator-channel stderr output and must not become extra JSON fields.

## Invariants

- JSON stdout remains a `DesktopCompanionStatus` object in both online and offline paths.
- If daemon and local observations disagree, local manager observation wins.
- A stale-daemon warning is emitted without changing the DTO schema.
- The CLI does not reclassify supervisor or observed state.

## Verification

- Unit tests cover equal online/local status and stale-daemon disagreement selection.
- Focused plugin CLI tests must pass.
- Touched-file terminology audit must pass.

## Results

- `cargo test -q cli::commands::groups::plugin` passed with 4 focused tests.
- `cargo test -q protocol::companion_contract` passed.
- `cargo test -q daemon::plugins::companion` passed.
- `git diff --check` passed.
- Touched-file terminology audit passed.
