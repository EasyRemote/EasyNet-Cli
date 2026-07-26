Semantic invariants:
- Descriptor-bound dispatch without a registered descriptor proof fails closed.
- Missing call_mode fails before catalog lookup.
- Owner mismatch fails before route or catalog probing.
- Local catalog misses must not probe remote state.
- Remote owner offline remains typed distinctly from ability absence.
- FFI does not own runtime business decisions; it only performs ABI/DTO translation.

Safety invariants:
- Error projections must preserve NotFound, OwnerOffline, RuntimeOwnerUnavailable, InvalidRequest, and InvalidCatalogPayload semantics.
