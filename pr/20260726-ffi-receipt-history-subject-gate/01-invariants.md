Semantic invariants
===================

- A receipt-history descriptor is only useful for a tuple that admission can
  authorize.
- Descriptor resolution must not hand products a descriptor for a tuple shape
  that canonical receipt-history admission rejects.
- Runtime-state reads are user-owned resource subjects, not target-device
  subjects.

Safety invariants
=================

- All-zero principal placeholders fail before daemon I/O.
- Retired session subjects fail before daemon I/O.
- Provider family mismatch still fails before catalog lookup.

Boundedness invariants
======================

- The check is local and deterministic; it does not dial the daemon or query
  directory state.
- The FFI resolver remains a descriptor projection seam, not an authority minting
  path.
