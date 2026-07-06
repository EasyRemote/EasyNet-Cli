Signer workflow acquisition

Intent:
- Converge signer acquisition on the SDK runtime model.
- Let IdentityClient obtain daemon-authorized signer handles and bind them to Runtime Core signature providers.
- Keep product callers away from manual signer-handle composition while preserving existing signing interfaces.

SPEC alignment:
- Invocation submissions require daemon-authorized signer material before submission.
- SDK facades own typed daemon profile DTOs and retry/error mapping.
- Product-specific keyring policy and daemon key storage remain outside the SDK.
