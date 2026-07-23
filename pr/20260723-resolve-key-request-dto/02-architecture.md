# Architecture

`FederatedKeyResolver` owns the admission decision tree and signer/caller
context. It does not own the JSON request schema of peer hub abilities.

`federation_wrappers::ResolveKeyRequest` owns the request DTO, optional
presented-key projection, and deterministic JSON encoding used by outbound
peer-hub invocation.
