# Authority C ABI Core Architecture

## Core Object

`daemon::invocation::admission::authority_metadata` owns the daemon SDK core
projection for:

- `DelegationPayload`
- `SessionAuthorityPayload`
- canonical signing material
- signed wire metadata materialization

Admission and FFI use this module instead of creating language-local authority
payload grammar.

## ABI Projection

`src/ffi/authority` exports four pure helper functions:

- `easynet_authority_prepare_delegation`
- `easynet_authority_materialize_delegation`
- `easynet_authority_prepare_session`
- `easynet_authority_materialize_session`

These functions do not require a daemon handle because they do not contact the
daemon. They are still SDK core ABI functions because they project the daemon's
admission metadata contract.

## Signing Boundary

The caller receives canonical bytes and hash, signs the bytes with its signer,
then returns only signature bytes/base64 to materialize metadata. Private key
material is rejected if accidentally included in request metadata.
