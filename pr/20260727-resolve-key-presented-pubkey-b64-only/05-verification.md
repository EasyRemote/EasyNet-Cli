Verification
============

Completed:

- `cargo test resolve_key --lib` — pass, 16 tests.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — pass.
- `bash tools/scripts/check-architecture-convergence.sh` — pass.
- `bash tools/scripts/check-daemon-invocation-migration.sh` — pass.
- `bash tools/scripts/check-daemon-latest-input-boundary.sh` — pass.
- `bash tools/scripts/check-system-ability-retired-aliases.sh` — pass.
- `cargo fmt --check` — pass.
- `git diff --check` — pass.
- `codegraph sync .` — pass.

Evidence:

- `ResolveKeyRequest` no longer has `presented_pubkey_hex`.
- `handle_resolve_key` and `dispatch_federation_resolve_key` no longer repair hex pins to base64.
- `resolve_key_request_rejects_retired_presented_pubkey_hex` proves the retired field fails closed.
- `federation.resolve_key` descriptor and daemon catalog schema expose `presented_pubkey_b64`.
