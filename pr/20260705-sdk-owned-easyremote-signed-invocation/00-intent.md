# SDK-owned EasyRemote signed Invocation transport

## Objective

Add an SDK-owned EasyRemote signed unary Invocation path over Runtime Core
prepare/sign/submit/await/close-handle semantics.

## Boundary

- Axon and the daemon remain responsible for canonical signing material,
  signature verification, admission, and receipt production.
- The SDK owns the facade state machine that prepares, signs through an SDK
  `Signer`, submits the signed Invocation, awaits the result, and releases the
  Invocation handle.
- EasyRemote may map the SDK error taxonomy into its public errors, but must not
  own signing material, signature algorithms, or raw handle lifecycle.

## Non-goals

- Do not add unsigned fallback for signed calls.
- Do not implement a new signer or signature algorithm in EasyRemote.
- Do not fabricate URA values.
- Do not edit `docs/spec/daemon-sdk-requirements-v1.md`.
