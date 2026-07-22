# Architecture

## Root abstraction defect

The FFI invocation parser treated the complete tuple as syntactic JSON rather than a canonical runtime boundary. That left authority/subject consistency to the downstream daemon admission path and made public-product errors appear as route, signer, or internal descriptor failures.

## Clean target

`InvocationJson::parse` owns FFI public-ingress validation:

- required tuple fields;
- canonical URA parse;
- all-zero placeholder rejection;
- authority metadata parser/projection using daemon-owned authority metadata logic;
- no subject defaulting and no fallback to callee/device.

Daemon admission remains the cryptographic authority. FFI validation is an early semantic gate only.
