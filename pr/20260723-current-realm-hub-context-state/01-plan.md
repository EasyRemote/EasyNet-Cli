# Current-realm hub invocation context convergence

## Goal

Remove credential-error collapse from current-realm Hub system ability dispatch.
The call-signaling path may fall back to the local voice ability only when the
machine is genuinely unpaired. Existing malformed or incomplete credentials are
authority-state corruption and must fail closed before a local fallback can run.

## Root abstraction problem

`invoke_current_realm_hub_system_ability()` currently uses:

```rust
let Ok(creds) = load_credentials() else { return Ok(None); };
```

This collapses multiple states into "Hub invocation unavailable":

- missing credentials file: unpaired local machine, safe to return `None`;
- malformed credentials file: corrupt authority state, must surface;
- incomplete credentials file: invalid caller identity, must surface.

Because `call.rs` treats `None` as permission to invoke the local voice ability,
malformed authority state can accidentally bypass realm-Hub signaling and run a
second product path. That is exactly the kind of product fallback that makes a
correct protocol hard to trust.

## Invariants

1. Missing credentials file remains `Unavailable`/`None` for public behavior.
2. Malformed/incomplete credentials propagate as errors.
3. Hub system dispatch derives `hub_ura` and caller device URA only from
   validated credentials.
4. `invoke_current_realm_hub_system_ability()` must not call
   `load_credentials()` directly.
5. No `let Ok(creds)` collapse may exist in the Hub context path.

## Boundary proof

```text
voice/call CLI signaling
  -> CurrentRealmHubInvocationContext::resolve()
       - load_credentials_optional()? == None => Unpaired
       - Some(validated credentials) => Ready { hub_ura, caller_ura }
       - Err => authority-state error
  -> RemoteSystemInvocationIssuer only when Ready
  -> otherwise local product voice fallback only for Unpaired
```

The fallback remains product-level behavior for unpaired local development, not
a recovery path from broken canonical identity state.

## Verification plan

- targeted Rust tests for missing, valid, malformed, and incomplete credentials;
- targeted shell gate preventing `load_credentials()` / `let Ok(creds)` collapse
  in `remote_system_ability.rs`;
- script-check wrapper for the gate;
- `cargo fmt --check`;
- `git diff --check`;
- `check-canonical-runtime-convergence-v2.sh`;
- `check-architecture-convergence.sh`;
- `codegraph sync/status`.

