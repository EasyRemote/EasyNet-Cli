# Call create participant identity state convergence

## Goal

Remove implicit credential-error fallback from `easynet call create`
participant identity selection. Missing credentials may still use the local
hostname for unpaired local development, but malformed or incomplete
credentials must fail closed instead of silently creating a call under a
hostname participant.

## Root abstraction problem

`run_create()` currently derives `participant_id` with:

```rust
load_credentials().ok()
  .map(|creds| creds.node_id)
  .filter(...)
  .unwrap_or_else(hostname)
```

That collapses three materially different states:

- validated device credentials: use the device node id;
- missing credentials file: unpaired local product state, use hostname;
- malformed/incomplete credentials: corrupt caller identity state, must error.

Because `call create` participates in voice signaling, silently hiding corrupt
credentials creates a second product identity path that bypasses canonical
identity readiness.

## Invariants

1. Valid credentials produce the credential node id as participant id.
2. Missing credentials produce a non-empty hostname participant id.
3. Existing malformed credentials are errors.
4. Existing incomplete credentials are errors.
5. `call.rs` must not use `load_credentials().ok()` or `unwrap_or_else(hostname)`
   to hide credential errors.

## Boundary proof

```text
call create
  -> CallCreateParticipantIdentity::resolve()
      - load_credentials_optional()? == Some(validated) => DeviceNode(node_id)
      - load_credentials_optional()? == None => UnpairedHostname(hostname)
      - Err => authority-state error
  -> voice.create_call args participant_id
  -> invoke_call_signaling(...)
```

This keeps local unpaired ergonomics while removing malformed authority-state
repair at the product ingress.

## Verification plan

- targeted `CallCreateParticipantIdentity` unit tests;
- boundary shell gate and self-test;
- script-check wrapper;
- `cargo fmt --check`;
- `git diff --check`;
- `check-canonical-runtime-convergence-v2.sh`;
- `check-architecture-convergence.sh`;
- `codegraph sync/status`.

