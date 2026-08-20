# Authority self stream admission convergence

## Goal

Allow the realm authority to open authority-owned stream abilities through self-authority, without minting a user/session authority carrier.

## Scope

- `src/daemon/invocation/admission/policy_engine.rs`
- `src/daemon/invocation/admission/policy_gate.rs`

## Validation

- `cargo test authority_self_stream --lib`
