# SDK access-control revoke actor boundary

## Intent

- Close the SDK-side access-control revoke mutation boundary so invalid audited mutations fail before provider wire dispatch.
- Keep daemon public behavior unchanged: daemon already requires canonical `actor_ura`; SDK providers now mirror that boundary instead of emitting incomplete requests.

## Boundary

- Go and Python provider facades own request validation before invoking EasyNet provider abilities.
- Daemon remains the authoritative mutation/audit owner and still validates `actor_ura` at the runtime boundary.

## Expected effect

- Effect convergence: missing actor identity is rejected once, deterministically, before any mutation wire call.
- Architecture cleanliness: provider facades stop treating audited identity as optional transport decoration.

## Verification plan

- Go unit test for valid, missing, and scalar `actor_ura` on access-control revoke.
- Python unit test for the same request boundary.
- Existing architecture convergence gates must remain clean.
