# Invariants

- Resolver owns route-negative semantics; admission maps typed resolver facts to transport status.
- Human diagnostic detail is not a semantic authority.
- `NXDOMAIN` absence and owner-offline are distinguishable without substring inspection.
- Route-negative outcomes remain deterministic and auditable.
- No legacy fallback path should preserve old message-string behavior.
