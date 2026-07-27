# Device-only runtime binding convergence

## Goal

Close the lifecycle fork where `easynet runtime start` treats every device
credential as user-bound even when the canonical Hub URA federation join
produces an explicit device-only runtime credential.

## Concrete failure

Clean-room reproduction after purging local state:

1. Start a local Hub daemon with a valid dev CA/server certificate.
2. Join a device with `easynet device join easynet:///r/localhost/authority`.
3. Start the joined device daemon.

Observed failure:

```text
Hub URA join lineage detected; skipping backend HTTP credential verification.
Error: sync local agent owner projection

Caused by:
    credentials file is missing user_id — run `easynet join <token>` to re-pair
```

## Root abstraction problem

`RuntimeUserBinding` already models the credential as either:

- `Bound { user_ura }`
- `Unbound { reason }`

but start lifecycle still has three direct user-bound assumptions:

1. local agent owner projection bootstrap is unconditional;
2. daemon ready validation always requires `paired_user_runtime_signer`;
3. startup presentation always renders a user-root pages URL and user welcome.

That makes a product user principal a hidden lifecycle requirement for a
device-only federation credential. The fix is not to synthesize a placeholder
user and not to add compatibility fallback; the fix is to make the runtime
principal binding state the single input to these branches.

## Invariants

1. Token-paired credentials remain fail-closed if they do not carry a concrete
   user id and username.
2. Bound credentials must still require `paired_user_runtime_signer` before
   `runtime.json` is published.
3. Device-only federation credentials must not mint or require a User signer.
4. Device-only federation credentials must not bootstrap user-owned local agent
   projections.
5. Device-only startup output must not render user-root Pages URLs.
6. Daemon ready capability flags must describe capabilities actually proven by
   boot, not desired product assumptions.

## Implementation steps

1. Thread `RuntimeUserBinding` through CLI start preflight and readiness.
2. Keep `RuntimeStartRequest::device` strict by default; add an explicit
   builder for device-only starts that do not require the paired-user signer
   capability.
3. Refactor local agent bootstrap behind a binding-aware method.
4. Refactor daemon boot signer registration to publish
   `paired_user_runtime_signer` only for bound credentials.
5. Add tests for bound and unbound lifecycle paths.
6. Re-run clean-room Hub URA join/start and convergence gates.
