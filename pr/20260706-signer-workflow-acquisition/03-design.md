Design:
- Add IdentityClient AcquireSigner/acquire_signer as a narrow facade.
- Validate provider presence before daemon transport calls.
- Delegate handle retrieval to the existing signer endpoint.
- Construct the existing Runtime Core Signer object with the validated daemon handle and supplied provider.
- Keep SignerHandle policy/provenance validation in one place.
