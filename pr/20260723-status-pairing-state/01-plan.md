# Runtime status pairing state

## Goal

Make `easynet runtime status` distinguish three pairing states before rendering operator diagnostics:

1. paired with complete credentials;
2. unpaired because credentials are absent;
3. invalid because an existing credentials file is malformed, incomplete, or semantically rejected.

## Root abstraction problem

The status command currently uses `if let Ok(creds) = load_credentials()`. That collapses missing credentials and invalid credentials into the same "not paired" output. After signer custody and descriptor-bound invocation cutover, this is a product-facing architecture defect: invalid identity state can look like normal unpaired setup, hiding the reason that caller signer or authority admission later fails.

## Invariants

1. Existing valid credentials render the same pairing facts as before.
2. Missing credentials remain source-compatible: status still reports "not paired".
3. Existing invalid credentials must not be rendered as "not paired".
4. Status rendering must be a pure projection over an explicit state object, not procedural error swallowing.
5. Regression must be caught by a targeted boundary gate.

## Boundary proof

- `load_credentials_optional()` is the canonical persistence API for the distinction required here: `Ok(None)` means absent, `Err` means invalid existing state.
- The CLI status layer only renders pairing state; it does not repair credentials, provision keys, or select invocation identities.
- The runtime lifecycle/status blocks are unchanged. This iteration only removes the pairing diagnostic collapse.

## Verification plan

1. Add Rust unit coverage for paired, unpaired, and invalid pairing-state projection.
2. Add a shell boundary gate forbidding `status.rs` from calling `load_credentials()` or using `if let Ok` credential collapse.
3. Wire the gate into `script_checks` and canonical convergence v2.
4. Run targeted tests, formatting, architecture gates, and codegraph sync.
