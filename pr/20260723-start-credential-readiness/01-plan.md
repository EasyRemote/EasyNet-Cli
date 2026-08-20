# Start credential readiness

## Goal

Make `easynet start` distinguish credential readiness before daemon boot:

1. ready credentials;
2. missing credentials;
3. invalid existing credentials.

## Root abstraction problem

`load_and_verify_credentials_with` currently uses `let Ok(creds) = load_credentials() else { ... }`, which collapses absence, malformed JSON, incomplete fields, retired fields, and semantic rejections into the same "no credentials" branch. After key-custody and signer-readiness cutover, this hides invalid identity state and sends operators toward pairing setup rather than state repair.

## Invariants

1. Missing credentials preserve existing public behavior: `easynet start` explains pairing and refuses device-daemon start.
2. Invalid existing credentials must not be described as missing.
3. Hub credential verification must only run after credentials are structurally valid.
4. Daemon-native Hub URA join lineage remains unchanged.
5. Revoked credentials behavior remains unchanged: validated credentials can still be deleted after Hub revocation.

## Boundary proof

- `load_credentials_optional()` already encodes the persistence distinction: absent credentials are `Ok(None)`; invalid existing credentials are `Err`.
- The start command owns boot readiness presentation; it must not repair credentials or provision keys.
- The new readiness state is local to start preflight and does not change public CLI arguments or daemon runtime APIs.

## Verification plan

1. Add tests for readiness projection and invalid credentials fail-closed before verifier execution.
2. Add a shell boundary gate forbidding `load_credentials()` collapse inside start preflight.
3. Wire the gate into `script_checks` and canonical convergence v2.
4. Run targeted tests, formatting, canonical gates, and codegraph sync.
