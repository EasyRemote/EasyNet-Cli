# Invariants

- A pending application rebind closes over application identity and the exact
  window-id set.
- ScreenCaptureKit is authoritative for the replacement generation's surface
  layout and union geometry.
- The candidate is committed only after the replacement capture plan starts.
- Candidate resolution/preparation failure supersedes only the candidate.
- Superseding preserves the committed binding and media-source epochs and
  restores the lifecycle from `Rebinding` to the active transport projection.
- Candidate failure emits `TARGET_REBIND_SUPERSEDED` with `recoverability=continue`.
- Only independently observed target loss, permission revocation, or rebind
  deadline expiry may project target/media loss.
- A rejected or stale commit restores the old capture output generation.
