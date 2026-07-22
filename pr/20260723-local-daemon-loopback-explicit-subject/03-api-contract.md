# API Contract

## Public API

`invoke_local_ability(ability, args)` remains available and keeps the same
signature.

## Internal request contract

- `function_name` must be non-empty.
- `payload_json` is forwarded unchanged.
- `callee_ura` for generic daemon-local loopback remains the control-plane
  daemon identity.
- `subject_ura` must be explicitly resolved before creating the loopback tuple.

## Error contract

- If daemon identity is unavailable, local ability invocation fails with the
  existing identity-readiness error from `local_daemon_ura`.
- If the explicit subject is malformed, tuple construction fails before gRPC
  invocation.
