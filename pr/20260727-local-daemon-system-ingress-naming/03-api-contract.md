# API Contract

## Internal API

Only crate-internal Rust names change. Public command, FFI, SDK, and protobuf surfaces do not change.

## Behavior

- Route names are still sent as route names for daemon resolution.
- Caller/callee/subject/nonce/causal context remain inspectable before dispatch.
- No compatibility aliases are kept for the retired internal names.
