# Hosted Agent User-ID Owner Cutover

## Goal

Remove the username-owned hosted Agent URA branch from daemon bootstrap so
LocalRuntime, Hub publication, and namespace resolution agree on one immutable
owner identity.

## Invariants

1. `BootstrapPlan` uses `user_id` as the hosted Agent owner segment.
2. `BootstrapPlan` does not carry `username`; display names are not authority
   facts.
3. Existing local-agent rows are reused only when their Agent URA owner segment
   matches the current immutable `user_id`.
4. Stale username-owned rows are pruned during post-join bootstrap and reminted
   under the canonical user-id owner.
5. Hosted-agent advertise prelude derives its owner segment from the paired
   runtime User URA and rejects device-only/federation-native credentials
   instead of falling back to a username or environment override.
6. This is EasyNet-Cli daemon projection policy. The SDK canonical runtime model
   remains product-neutral.

## Boundary Proof

Hub admission validates hosted Agent publication against the caller Device's
owner. Username slugs are mutable display/product aliases and cannot prove that
the Agent URA belongs to the Device owner. Minting hosted Agent URAs under
`user_id` makes the LocalRuntime row, advertised authority root, and backend
resolver target the same namespace owner.

The bidi session prelude uses `Credentials::runtime_user_binding()` and parses
the resulting User URA before extracting `user_id`. That keeps the advertise
path on the same canonical identity projection as daemon bootstrap and removes
the previous dev-only username/environment owner branch.

## Verification

- `cargo test -q bootstrap_mints_ura_for_each_enabled_profile`
- `cargo test -q hosted_agent_ura_owner_prefix_is_user_id_not_username`
- `cargo test -q hosted_agent_owner_segment`
- `cargo test -q start_agent_materialize_syncs_hosted_ura_and_default_chat_manifest`
- `cargo test -q stop_agent_by_ura_removes_joined_hosted_mapping`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
