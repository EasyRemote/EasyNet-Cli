# Trust anchor strict clean start

## Intent

Fix device connection degradation caused by stale local trust data without adding
a compatibility path for obsolete Hub identities. New runtime state must use the
canonical authority URA (`easynet:///r/<realm>/authority`) only.

## Invariants

1. Trust-anchor TOML schema and row semantics remain strict.
2. A malformed existing trust anchor must not produce a false-ready daemon.
3. Missing trust-anchor files may still boot as an empty trust set for first-run
   local development and tests.
4. Runtime Hub identity projection uses `core::ura::hub_ura`, which delegates to
   Axon's authority URA builder.
5. Production source must not hand-write or persist `easynet:///r/<realm>/hub`.

## Boundary proof

- Strict parsing stays in `RealmTrustAnchor::load_or_empty` and
  `try_load_strict`.
- Boot owns startup health, so malformed trust-anchor errors propagate from
  `load_trust_anchor_from` instead of being converted to an empty runtime trust
  set.
- Local cleanup is operational state reset, not an SDK/runtime compatibility
  layer.

## Verification

- CodeGraph/rg scan for `/hub` URA construction and stale Hub identity wording.
- Focused trust-anchor and boot tests.
- `cargo check --bin easynet --bin easynet-daemon`.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`.
- Clean local runtime state, rebuild, and verify device-mode `easynet start`
  does not falsely boot without credentials; a new environment must run
  `easynet device join <token>` before reaching `FRONTEND_CONNECTED`.
