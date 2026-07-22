# Reset credential state

## Goal

Make `easynet reset` distinguish local credential state before confirmation and best-effort remote revoke:

1. paired credentials are available and can derive the device URA;
2. credentials are absent;
3. an existing credentials file is invalid.

## Root abstraction problem

`reset.rs` currently calls `load_credentials().ok()` for the confirmation prompt and `if let Ok(creds) = load_credentials()` for remote revoke. Both collapse an invalid existing credentials file into the same branch as absence. Reset is a cleanup command and must still be able to delete invalid local credentials, but it must not pretend that corrupt identity state was simply absent.

## Invariants

1. Reset can still remove missing or invalid local credential state when the runtime lifecycle guard allows it.
2. Remote revoke only runs when complete paired credentials exist and a device URA can be derived.
3. Invalid credentials are surfaced as invalid state and never rendered as `<no credentials on disk>`.
4. No prompt/revoke path may call `load_credentials().ok()` or `if let Ok(load_credentials())`.
5. Runtime projection fail-closed behavior remains unchanged.

## Boundary proof

- `load_credentials_optional()` provides the exact state split needed by reset.
- Reset owns local cleanup; it does not repair invalid credentials or synthesize a device URA from partial fields.
- Skipping remote revoke for invalid credentials is explicit and observable. It is not a compatibility fallback because local cleanup remains the command's primary authority and remote revoke requires validated paired identity.

## Verification plan

1. Add unit tests for paired/missing/invalid reset credential state.
2. Add a reset test proving malformed credentials are deleted under `--yes` rather than misclassified through prompt/revoke.
3. Add a boundary script forbidding credential error collapse in `reset.rs`.
4. Run targeted tests, formatting, convergence gates, and codegraph sync.
