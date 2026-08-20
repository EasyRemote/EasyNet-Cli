## Boundary Proof

`ErrDaemonKeyServiceNotFound` and `ErrDaemonKeyServiceUnavailable` are the
semantic error owners for daemon-custodied signing identity resolution. The
runtime identity facade may expose source-compatible names, but those names are
not independent runtime capability states and must not appear in canonical
capability inventory.

The proof target is therefore the SDK public-surface policy, not transport
logic:

- if the Go aliases remain exported, they must be listed under
  `non_canonical.languages.go`;
- each alias must carry `legacy_quarantine` metadata;
- the replacement must point at `capability_inventory.runtime_identity`;
- the generated parity matrix must no longer list those aliases as canonical
  runtime identity evidence.

This keeps source behavior stable while removing the false canonical owner.
