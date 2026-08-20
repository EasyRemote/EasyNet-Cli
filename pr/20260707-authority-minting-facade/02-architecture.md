# Authority Minting Architecture

## Layering

- Axon owns canonical authority payload shape, signature verification, and
  admission semantics.
- EasyNet-Cli daemon/native layer owns product authority policy and may wrap
  Axon helpers or daemon keyring execution.
- Go/Python SDK expose typed request/projection clients over a transport
  boundary.
- EasyNet backend and EasyRemote call SDK clients, not raw Axon helpers.

## Module Boundaries

- `sdk/go/authority.go`: typed authority DTOs, request validation, transport
  interface, client facade.
- `sdk/python/easynet_sdk/authority.py`: matching Python facade and request
  DTOs.
- Tests use memory transports to prove SDK behavior without embedding protocol
  canonicalization in the SDK.

## Boundary Proof

The facade accepts only typed request DTOs and returns only typed SDK authority
projections. The metadata bytes are still produced by the transport/provider
boundary. This removes product-layer direct Axon imports as an architectural
requirement while preserving Axon as the semantic owner.
