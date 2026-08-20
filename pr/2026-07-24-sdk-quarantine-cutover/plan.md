# SDK quarantine cutover hardening

## Goal

Close the remaining SDK public-surface compatibility window where process-local
signer fallback helpers could still be represented as `non_canonical` legacy
quarantine entries.

## Invariants

1. The canonical runtime SDK exposes signer custody only through explicit
   provider-backed signer abstractions.
2. Process-local signer helpers are not valid public SDK exports in any graph
   section, including legacy/non-canonical inventory.
3. A public API manifest can describe canonical runtime concepts, but must not
   keep compatibility quarantine as a route for retired fallback signer helpers.
4. Go, Python, Java, Node, Swift, Rust and C ABI inventory rules remain
   language-parity rules rather than language-specific exceptions.

## Boundary proof

- `sdk/conformance/canonical-public-api.json` already carries empty
  `legacy_quarantine` sections. The implementation should make that empty state
  structural by rejecting fallback signer symbols in `non_canonical`, not merely
  validating their quarantine reason.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` is the SPEC v2 gate
  boundary and must reject fallback signer helpers in both canonical and
  non-canonical public graph sections.
- `sdk/conformance/sdk_concepts.py` is the manifest/schema boundary and must no
  longer approve process-local signer quarantine entries as a valid future
  state.

## Verification plan

1. Run SDK conformance self-test for public API concepts.
2. Run SPEC v2 gate.
3. Run architecture convergence gate.
4. Check formatting/no unintended worktree residue.

## Decision log

- Treat process-local signer fallback as removed, not quarantined. The SDK model
  is explicit signer/provider backed; keeping a quarantine reason for the old
  helper family weakens the architecture by preserving a public compatibility
  state the SPEC no longer requires.
