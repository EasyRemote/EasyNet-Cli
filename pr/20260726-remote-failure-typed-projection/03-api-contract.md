# API Contract

## Inputs

- `context`: stable call-site context for diagnostics.
- `raw_error`: diagnostic text from remote transport or legacy frame.
- `failure`: optional typed `SessionFailure`.

## Outputs

- With `failure`: gRPC `Status` code and message are derived from `failure.code` and `failure.status_detail()`.
- Without `failure`: gRPC `Status` is `Unavailable` with `REMOTE_FAILURE_UNTYPED`, preserving bounded diagnostics without semantic promotion.

## Error rules

- `AUTHORITY_DENIED`, `POLICY_DENIED`, and signature classes require typed failure facts.
- `DESCRIPTOR_OWNER_OFFLINE` and `CALLER_SIGNER_UNAVAILABLE` require typed failure facts.
- Raw strings containing keyring details are redacted in the untyped projection.
