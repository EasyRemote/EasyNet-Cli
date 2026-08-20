# User Binding Token Wire Strictness

## Goal

Make the federated user-binding token wire shape fail-closed. `UserBindingToken` is both the signed canonical material source and the external JSON token payload; unknown token fields must be rejected instead of silently ignored by serde.

## Invariants

- The existing signed fields remain unchanged:
  - `source_realm`
  - `source_user_ura`
  - `source_user_pubkey`
  - `target_realm`
  - `issued_at_ms`
  - `nonce`
  - `signature`
- Unknown fields are rejected at decode time.
- Canonical bytes and signature verification semantics remain unchanged for valid tokens.
- No compatibility fallback accepts old/permissive token extensions.
- Response DTO boundaries from `user_binding_projection` remain unchanged.

## Boundary Proof

The token is signed over exactly the canonical fields in `canonical_user_binding_bytes`. Allowing unknown JSON fields creates an unsigned side-channel on the same payload object. `#[serde(deny_unknown_fields)]` closes that side-channel without changing the canonical byte algorithm or the valid public wire shape.

## Verification Plan

- Add focused Rust test proving `UserBindingToken` rejects unknown fields.
- Extend SPEC v2 gate to require token strict serde and a regression test.
- Run user-binding signature/consume focused tests plus SPEC gates, fmt, check, diff, and codegraph.

## Implementation Delta

- Added `#[serde(deny_unknown_fields)]` to `UserBindingToken`.
- Added `token_wire_shape_rejects_unknown_fields` to prove unsigned extension fields are rejected before token use.
- Added SPEC v2 gate coverage for:
  - strict token serde boundary
  - exact signed/public token fields
  - absence of permissive token extension/default patterns
  - regression test evidence
- Added SPEC v2 self-test fixture for a permissive legacy token shape.

## Verification Results

- `cargo test -q --features axon-pb token_wire_shape_rejects_unknown_fields --lib`
- `cargo test -q --features axon-pb round_trip_serde_preserves_all_fields --lib`
- `cargo test -q --features axon-pb signed_token_verifies_with_correct_pubkey --lib`
- `cargo test -q --features axon-pb consume_federate_user_token_happy_path --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `cargo check -q --features axon-pb`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
- `/Users/macbook.silan.tech/.local/bin/codegraph affected src/daemon/keyring/user_binding_chain.rs tools/scripts/check-canonical-runtime-convergence-v2.sh`

## Follow-up Seam

The token still exposes raw byte vectors for public key, nonce, and signature. That is currently public wire-compatible, but a later SDK-facing iteration should evaluate typed byte-array newtypes or language-neutral generated schema vectors once downstream consumers are ready.
