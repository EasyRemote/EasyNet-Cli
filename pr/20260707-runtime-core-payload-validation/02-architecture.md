# Architecture

## Root Abstraction Problem

The raw payload carrier is part of the canonical Runtime Core draft shape. If malformed raw bytes reach transports, individual submit paths can disagree about whether the draft is valid.

## Target Architecture

- Keep raw-carrier validation in the same builder inspection path as tuple completeness and nonce validation.
- Reuse existing base64 validation helpers where available.
- Keep transport, signing, and profile clients downstream of one validated draft object.

## Module Boundaries

- `sdk/go/invocation.go`: Go draft construction.
- `sdk/python/easynet_sdk/invocation.py`: Python draft construction.
- Existing invocation tests pin the invalid-carrier behavior.
