# Seven Axes Hardening Plan

## Scope

Harden the current `seven-axes-p0-landing-v1` branch before merge by removing
contract drift, credential-coupled subject construction, and procedural control
plane seams that fail strict verification.

Ignored by request: `examples`, `document`, and `Frame` changes.

## Non-Negotiable Invariants

1. Receipt facts keep stable, typed meanings. `runtime_env` identifies the
   execution environment only; descriptor proof state must be carried as its own
   fact.
2. Mission/EAL child invocations always expose the Axon invocation tuple fields.
   Subject derivation must not fail merely because a local-only device is not
   paired.
3. Teach/acquire/forget grant transitions are deterministic and auditable.
   Transaction state must be represented as domain state, not loose primitive
   argument groups.
4. Control-plane registration functions accept typed requests so schema,
   authority, implementation, and descriptor facts stay coherent.
5. Strict clippy with `-D warnings` is a merge gate for changed Rust code.

## Boundary Proof

- Axon-owned semantics: receipt fact names, descriptor proof binding, invocation
  tuple shape, descriptor version/schema/implementation facts.
- EasyNet-Cli daemon-owned semantics: local credential lookup, local mission
  subject construction, device-hosted agent dispatch, teach/acquire persistence.
- CLI facade-owned semantics: presentation and command argument mapping only.

## Implementation Order

1. Repair proof fact construction so descriptor proof state is not encoded in
   `runtime_env`.
2. Introduce an explicit local daemon subject source that permits local-only
   mission execution without weakening paired-device identities.
3. Replace oversized primitive APIs with typed request objects where clippy
   currently flags control-plane and grant APIs.
4. Clean mechanical clippy findings without suppressing lints.
5. Re-run focused e2e checks, no-default build, strict clippy, and no-run build.

## Validation Matrix

- `CARGO_TARGET_DIR=target/review-clippy cargo clippy --features axon-pb --lib --tests -- -D warnings`
  passed.
- `CARGO_TARGET_DIR=target/review-verify cargo test --test seven_axes_w3_usage_e2e -- --nocapture`
  passed.
- `CARGO_TARGET_DIR=target/review-verify cargo test --test seven_axes_w1_discover_e2e --test seven_axes_w2_watch_e2e --test seven_axes_w3_teach_learn_e2e -- --nocapture`
  passed.
- `CARGO_TARGET_DIR=target/review-verify cargo test --test script_checks -- --nocapture`
  passed.
- `CARGO_TARGET_DIR=target/review-verify cargo test local_mission_subject_owner_tests --features axon-pb -- --nocapture`
  passed.
- `CARGO_TARGET_DIR=target/review-verify cargo test teach_grants --features axon-pb -- --nocapture`
  passed.
- `CARGO_TARGET_DIR=target/review-no-default cargo check --no-default-features`
  passed.
- `CARGO_TARGET_DIR=target/review-verify cargo test --no-run`
  passed.
- `CARGO_TARGET_DIR=target/review-sdk cargo test receipt_proof_normalization_keeps_runtime_env_pure -- --nocapture`
  passed in the local EasyNet-Axon SDK path dependency.
