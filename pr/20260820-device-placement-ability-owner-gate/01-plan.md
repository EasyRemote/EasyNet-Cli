# Device placement Ability-owner gate

## Invariants

- `Device` is execution substrate and placement input only.
- Public `AbilitySelector` must accept callable owners only: `Agent`, `Service`, `SystemAgent`, `Authority`.
- Routing may accept `Device + public ability name` as a placement query, but must project it to a device-sponsored `SystemAgent` owner before executable descriptor resolution.
- Explicit Device-owned Ability URAs remain migration read-model artifacts and are refused at route/selector boundaries.

## Implementation plan

1. Reject `ability/device.*` in the core Ability selector instead of carrying a `"device"` owner kind.
2. Preserve legacy user-facing placement calls by projecting `device_ura + public ability` before `AbilitySelector` parsing.
3. Keep descriptor refs and explicit Ability URAs bound to their embedded owner; do not rewrite them.
4. Update architecture convergence gates and fixture self-test to encode the owner/placement split.

## Verification

- `cargo test -q ability_selector --features axon-pb`
- `cargo test -q route_selector --features axon-pb`
- `cargo test -q explicit_device_owned_ability_ura --features axon-pb`
- `cargo test -q route_resolver --features axon-pb`
- `cargo test -q device_placement --features axon-pb`
- `bash tests/scripts/test_check_architecture_convergence.sh`
