# Authority Minting API Contract

## Go

```go
type AuthorityTransport interface {
    MintDelegationProof(ctx context.Context, requestJSON []byte) ([]byte, error)
    MintSessionAuthority(ctx context.Context, requestJSON []byte) ([]byte, error)
}

type AuthorityClient struct { ... }

func NewAuthorityClient(transport AuthorityTransport) (*AuthorityClient, error)
func (c *AuthorityClient) MintDelegationProof(ctx context.Context, req DelegationRequest) (DelegationProof, error)
func (c *AuthorityClient) MintSessionAuthority(ctx context.Context, req SessionAuthorityRequest) (SessionAuthority, error)
```

## Python

```python
class AuthorityTransport(Protocol):
    def mint_delegation_proof(self, request_json: bytes) -> bytes: ...
    def mint_session_authority(self, request_json: bytes) -> bytes: ...

class AuthorityClient:
    def mint_delegation_proof(self, request: DelegationRequest) -> DelegationProof: ...
    def mint_session_authority(self, request: SessionAuthorityRequest) -> SessionAuthority: ...
```

## Errors

- Missing binding fields return `INVALID_ARGUMENT`.
- Transport failures are wrapped as SDK transport/profile errors.
- Malformed metadata projections return `INVALID_ARGUMENT`.

## Tenant Rules

The SDK does not infer tenant/realm policy. Realm and issuer authority remain
encoded in URAs and enforced below the transport boundary.
