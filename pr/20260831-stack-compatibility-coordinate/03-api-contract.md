# Tool contract

## `check-axon-lock.py`

- Default mode verifies CLI source metadata, Rust/Python resolution, Axon checkout HEAD, and every projected Axon contract fact.
- `--axon-root PATH` selects the exact checkout verified by default mode.
- `--lock-only` validates the closed lock schema without requiring an Axon checkout; workflows use this before checkout.
- `--github-output` writes the pinned revision and contract digest to `GITHUB_OUTPUT` for workflow composition.
- Exit non-zero on malformed schemas, unknown keys, non-exact revisions, contract drift, or dependency mismatch.

No mode writes the lock. Coordinate changes are explicit reviewed JSON edits after successful candidate evidence.
