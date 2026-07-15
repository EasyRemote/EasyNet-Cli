Invariants

1. `provider_routes/easynet-access-control-routes.v1.json` is the only editable
   source for AccessControl route ability names in this slice.
2. Generated route constants are internal to their language package/module; no
   new public API surface is introduced.
3. Existing Go constant identifiers, Python `_ABILITY_*` identifiers, and daemon
   `names::governance::*` identifiers remain valid aliases to preserve current
   internal call sites.
4. AccessControl state machines, permission request terminality, authority
   proof shape, admission explain projection, descriptor metadata, and receipt
   semantics do not change.
5. Remaining literal AccessControl route strings are allowed only in the
   manifest, generated files, user-facing diagnostic text, and tests/assertions
   that verify wire behavior.
6. Generator `--check` must fail when any generated binding is stale.

Boundary proof

- SDK ownership: Go/Python SDKs own language facades and provider invocation
  convenience, not the canonical route list. They consume generated constants.
- Daemon ownership: the daemon owns governance ability installation and
  AccessControl policy execution, but route spelling is shared with SDK
  providers through the manifest.
- Axon boundary: this change does not define or fork Axon invocation semantics;
  it only aligns daemon-owned ability names used inside complete Invocation
  calls.
