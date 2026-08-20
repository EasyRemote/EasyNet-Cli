# Decisions Log

## 2026-07-23

- Decision: remove descriptor resolver remote probing instead of making signer
  provisioning more permissive.
- Reason: descriptor lookup must be bounded catalog access. Hidden remote
  invocation inside lookup preserves a legacy fallback path and creates the
  exact product failure mode observed by the UI: descriptor miss becomes signer
  missing, owner offline, or timeout.

## 2026-07-23 Verification

- Decision: update both SPEC v2 and legacy architecture gates to reject the
  removed remote-probe path.
- Reason: leaving a gate that required `RemoteDescriptorCatalogProbe` would
  encode the legacy path as architecture, contradicting the bounded descriptor
  lookup invariant.
- Decision: delete the now-unreferenced typed remote submit wrapper in
  `remote_invoke.rs`.
- Reason: after descriptor probe removal it had no production caller; keeping it
  would preserve a compatibility surface for reintroducing hidden descriptor
  probes.
