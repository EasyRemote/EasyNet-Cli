# Mission ability ingress vocabulary convergence

## Goal

Remove legacy/compatibility-shaped vocabulary from the canonical mission ability ingress. `mission.run`, `mission.track`, and `mission.cancel` are descriptor-owned daemon abilities, not shims or fallback carriers.

## Invariants

- Mission/EAL remains daemon-owned composite ability implementation.
- `mission.run` still enters `MissionRunner` through the admitted envelope gateway.
- Strict JSON ingress remains fail-closed for unknown fields.
- Runtime behavior and public ability names are unchanged.

## Boundary proof

- `src/daemon/ability/builtins/automation/mission.rs` is the public ability handler for mission execution.
- Its boundary language teaches callers which path is canonical; retaining shim/fallback/legacy-carrier vocabulary suggests a second model where none should exist.
- `tools/scripts/check-mission-ability-vocabulary-boundary.sh` is the existing dedicated gate for this module's public vocabulary and is the correct place to extend the negative contract.

## Refactoring plan

1. Replace shim/fallback wording with descriptor/adapter vocabulary.
2. Rename negative test payloads from legacy-carrier to retired-field vocabulary.
3. Extend the mission ability vocabulary gate and self-test to reject reintroduction.

## Verification

- `bash tests/scripts/test_check_mission_ability_vocabulary_boundary.sh`
- `bash tools/scripts/check-mission-ability-vocabulary-boundary.sh`
- `cargo test automation::mission --lib`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo fmt --check`
- `git diff --check`
- `codegraph query "mission ability ingress vocabulary legacy shim fallback"`
