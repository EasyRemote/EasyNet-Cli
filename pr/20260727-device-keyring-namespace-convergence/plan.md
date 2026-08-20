# Device Keyring Namespace Convergence

## Goal

Remove the remaining internal owner-parameterized keyring registration seam that preserved the old self-alias shape. The public ability surface remains `device.keyring.*`; the implementation should express that keyring administration is a device-owned runtime capability, not an arbitrary owner-scoped agent family.

## Scope

- Refactor daemon keyring ability registration from `register_for_owner(owner)` to a device-owned registration API.
- Remove active-source references to the retired `legacy self alias` vocabulary in this boundary.
- Preserve public ability names and admission actions.
- Do not add compatibility aliases or fallback routes.

## Invariants

1. Keyring management abilities are registered only under `device.keyring.<verb>`.
2. Raw signing is not exposed as an Invocation ability.
3. Registration must not accept arbitrary owner strings.
4. Assembly tests must continue proving `device.keyring.*` names are present and `device.keyring.sign` is absent.
5. SPEC v2 and SDK product-neutrality gates must remain green.

## Boundary Proof

The keyring provider is daemon/device policy, not canonical SDK surface. Refactoring the registration function inside `src/daemon/keyring/abilities.rs` keeps the behavior in EasyNet-Cli daemon ownership while removing a generic owner seam that could recreate product-specific aliases.

## Verification Plan

- `cargo test keyring --lib`
- `cargo test build_registry_always_registers_key_service_abilities --lib`
- `cargo test published_catalogue_never_contains_placeholder_owner --lib`
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `codegraph query "legacy self alias device keyring register_for_owner" --limit 40`

## Verification Results

- `cargo test keyring --lib` — passed, 132 tests.
- `cargo test build_registry_always_registers_key_service_abilities --lib` — passed.
- `cargo test published_catalogue_never_contains_placeholder_owner --lib` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `bash tools/scripts/check-sdk-product-neutrality.sh` — passed.
- `codegraph sync .` followed by `codegraph query "legacy self alias device keyring register_for_owner" --limit 40` — keyring `register_for_owner` no longer appears; remaining `register_for_owner` hits belong to ledger/governance contract registration outside this slice.
