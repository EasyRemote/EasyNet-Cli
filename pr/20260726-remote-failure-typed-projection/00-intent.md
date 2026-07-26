# Intent

## Goal

Remove raw-string compatibility classification from remote invocation failure projection.

Remote unary and stream dispatch already carry typed `SessionFailure` facts. The dispatch layer must use those structured facts as the only semantic authority for remote failure classes such as authority denial, descriptor owner offline, route unavailable, or caller signer unavailable.

## Non-goals

- Do not change namespace route resolution semantics.
- Do not change the `SessionFailure` wire shape.
- Do not add compatibility parsing for older untyped remote frames.
- Do not change public gRPC status codes for typed failures.

## Acceptance criteria

- `status_from_remote_failure` no longer derives permission/routing/signer classes from raw error text when `SessionFailure` is absent.
- Typed `SessionFailure` still projects owner-offline routes as `Unavailable`.
- Typed `SessionFailure` still redacts keyring implementation details from caller signer failures.
- Untyped raw remote errors project as a bounded upstream failure instead of a guessed canonical class.
- Focused tests and convergence gates pass.
