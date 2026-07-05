# Architecture

## Layering

1. Rust `daemon::publication_contract` owns request validation, daemon system-ability carrier construction, and daemon result projection.
2. `src/ffi/publication` exports the contract through C ABI v4 functions.
3. Python `_cabi.CABIPublicationTransport` invokes the carrier through Runtime Core and projects the result with C ABI functions.
4. Python `PublicationClient` keeps its existing OOP facade and only validates local DTO completeness before delegating.

## Boundary Proof

- Axon remains the owner of URA and Invocation semantics; this slice only uses existing daemon SDK helpers and system Invocation construction.
- The SDK does not introduce a new lifecycle backend. The daemon system abilities remain the execution authority.
- No EasyRemote product code is added. This reduces EasyRemote's need for raw daemon system-ability carriers later.
