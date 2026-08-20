# Desktop Companion SDK Projection Invariants

1. Desktop companion lifecycle is local daemon control-plane behavior, not an Axon protocol capability.
2. SDK facades parse the same JSON DTO fields projected by `src/protocol/companion_contract.rs`.
3. Python and Go expose the same generic runtime concepts: companion status, list, and action result.
4. Daemon handles remain the lifecycle root; companion actions cannot be called without an attached daemon handle.
5. Unsupported transports fail explicitly instead of silently falling back to legacy control paths.
6. URA terminology is preserved; no alternate address terminology is introduced.
