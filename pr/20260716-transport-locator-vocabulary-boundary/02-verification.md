# Verification

- `bash tools/scripts/check-node-sdk-seam.sh`
- `bash tests/scripts/test_check_system_ability_retired_aliases.sh`
- `bash tools/scripts/check-sdk-ura-naming.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check -- tools/scripts/check-node-sdk-seam.sh tests/scripts/test_check_system_ability_retired_aliases.sh pr/20260716-transport-locator-vocabulary-boundary`

## 2026-07-16 Result

- PASS: `bash tools/scripts/check-node-sdk-seam.sh --self-test && bash tools/scripts/check-node-sdk-seam.sh`
- PASS: `bash tests/scripts/test_check_system_ability_retired_aliases.sh`
- PASS: `bash tools/scripts/check-sdk-ura-naming.sh --self-test && bash tools/scripts/check-sdk-ura-naming.sh`
- PASS: `tools/scripts/check-architecture-convergence.sh`
- PASS: `git diff --check -- tools/scripts/check-node-sdk-seam.sh tests/scripts/test_check_system_ability_retired_aliases.sh pr/20260716-transport-locator-vocabulary-boundary`
