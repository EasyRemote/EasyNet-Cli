# Federation receipt DTO convergence

## Intent

Remove legacy federation/trust DTO shapes that still surface during real
Docker media/bidi runtime paths after the canonical runtime convergence work.

## Boundary invariants

- Federation and trust payloads must use the current governed ability schema.
- Hub/device product paths must remain daemon Invocation paths, not alternate
  product-specific RPC compatibility paths.
- Receipt bodies must not carry retired fields that are rejected by canonical
  receipt validation.
- Identity registration payloads must name the canonical principal fields used
  by the receiving descriptor.

## Current evidence

- Docker media/bidi E2E passes, but daemon logs still show:
  - `identity.register_pubkey` rejecting legacy `agent_ura`.
  - `federation.heartbeat` receipt validation rejecting legacy
    `refreshed_owner_count`.

## Execution plan

1. Locate the authoritative ability descriptors and all payload builders.
2. Remove or rename legacy payload fields at the producer boundary.
3. Add/update regression checks so rejected fields cannot return.
4. Verify with targeted tests, SPEC v2 gate, and relevant Docker/product E2E.

## Verification log

- `cargo fmt --check`
- `cargo test -q handle_heartbeat_reports_registry_size --features axon-pb`
- `cargo test -q handle_heartbeat_renews_owner_projection_lease --features axon-pb`
- `cargo test -q handle_heartbeat_skips_unknown_owner --features axon-pb`
- `cargo test -q invoke_dispatches_federation_heartbeat --features axon-pb`
- `cargo test -q register_pubkey_request_encodes_principal_scoped_tuple --features axon-pb`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/docker-media-bidi-e2e.sh --skip-build --project easynet-media-bidi-codex --out-dir target/e2e/docker-media-bidi/codex-20260727-014723`
- `rg -n "refreshed_owner_count|hub_trust_sync_write_failed|federation_heartbeat_failed|AUTHORITY_DENIED|ABILITY_NOT_FOUND|descriptor_ref not found|NOT_FOUND|INTERNAL_ERROR|CANCELLED|Timeout expired" target/e2e/docker-media-bidi/codex-20260727-014723 || true`
  - No matches.
