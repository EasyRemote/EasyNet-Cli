# Node Prepare Sign Seam Intent

Implement the Node/TypeScript Runtime Core prepare/sign boundary described by
`docs/spec/daemon-sdk-requirements-v1.md`.

## Scope

- Add typed `PreparedInvocation`, `SigningMaterial`, `SignerPolicy`,
  `InvocationSignature`, and `SignedInvocation` DTOs.
- Make `RuntimeClient.prepare` return daemon/Axon-provided prepared material
  instead of anonymous JSON.
- Ensure `RuntimeClient.submitSigned` accepts only `SignedInvocation`.
- Add caller-signature attachment without SDK-side canonicalization.
- Declare Node evidence for shared prepare/sign/handle conformance cases covered
  by tests.

## Out Of Scope

- No local canonical Invocation byte construction.
- No Ed25519 private-key signing provider in Node.
- No local daemon signing provider or signer-handle acquisition in this slice.
- No daemon transport provider or C ABI bridge.
