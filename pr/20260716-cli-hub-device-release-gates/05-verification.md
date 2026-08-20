# Verification

Planned commands:

```text
bash tools/scripts/check-release-package-contract.sh
bash tests/scripts/test_check_release_package_contract.sh
bash tools/scripts/cli-hub-device-daemon-e2e.sh --self-test
bash tools/scripts/check-sdk-cutover-readiness.sh --self-test
bash tools/scripts/check-daemon-latest-input-boundary.sh
bash tools/scripts/check-architecture-convergence.sh
bash tests/scripts/test_check_architecture_convergence.sh
git diff --cached --check
```

Actual results:

```text
PASS bash tools/scripts/check-release-package-contract.sh
PASS bash tests/scripts/test_check_release_package_contract.sh
PASS bash tools/scripts/cli-hub-device-daemon-e2e.sh --self-test
PASS bash tools/scripts/check-sdk-cutover-readiness.sh --self-test
PASS bash tools/scripts/check-daemon-latest-input-boundary.sh
PASS bash tools/scripts/check-architecture-convergence.sh
PASS bash tests/scripts/test_check_architecture_convergence.sh
PASS bash -n packaging/release/dev-install-local.sh
PASS bash -n tools/scripts/check-sdk-cutover-readiness.sh
PASS git diff --cached --check
```

Full live E2E remains:

```text
bash tools/scripts/cli-hub-device-daemon-e2e.sh
```

It is intentionally not part of the cheap cutover self-test path because it
builds binaries, starts three daemons, creates TLS material, and performs load.
