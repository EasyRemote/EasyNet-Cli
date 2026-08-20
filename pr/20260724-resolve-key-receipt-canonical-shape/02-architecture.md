# Architecture

## Boundary

`src/daemon/federation/client/ability_contract.rs` owns the client-side typed contract for federation ability receipts. It must mirror the daemon/hub canonical wire response, not older product DTOs.

## Module ownership

- `src/daemon/federation/wire_contract.rs` defines the canonical resolve-key response shape.
- `src/daemon/invocation/dispatch/federation_wrappers.rs` produces that shape.
- `src/daemon/federation/client/ability_contract.rs` consumes that shape.

## Refactoring direction

Replace permissive partial parsing with a complete, strict DTO. This removes a hidden compatibility layer where old fields could coexist with ignored canonical facts.
