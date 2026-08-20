# Verification

Results:

- `cargo test route_negative` — passed.
- `cargo test resolver_owner_offline` — passed.
- `cargo test route_negative_owner_offline_is_route_unavailable_not_ability_absent` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `tools/scripts/check-architecture-convergence.sh` — passed.
- `codegraph sync`, `codegraph status`, and targeted `codegraph explore` — passed.

Notes:

- `cargo test resolver_owner_offline route_negative_owner_offline_is_route_unavailable_not_ability_absent` was rejected by Cargo because it accepts one filter argument; both filters were run separately.
