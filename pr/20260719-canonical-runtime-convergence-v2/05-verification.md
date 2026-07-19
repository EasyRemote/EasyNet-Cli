# Verification

Completed in this slice:

- `python3 scripts/checks/check_benchmark_baselines.py`
- `PATH="$HOME/.cargo/bin:$PATH" cargo bench --manifest-path sdk/rust/Cargo.toml --bench local_runtime_allocations -- --output sdk/rust/benches/baseline-v2.json`
- `PATH="$HOME/.cargo/bin:$HOME/go/bin:$PATH" tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `bash tools/scripts/check-downstream-sdk-consumer-cutover.sh`
- `bash tools/scripts/check-sdk-cutover-readiness.sh`
- `tools/scripts/build-linux-cli-artifact-bundle.sh --self-test`
- `tools/scripts/docker-two-node-easyremote-cli-e2e.sh --self-test`
- `tools/scripts/host-media-device-e2e.sh --self-test`
- `tools/scripts/host-media-device-e2e.sh --out-dir target/e2e/host-media-device/skip-check-2`
- `tools/scripts/docker-two-node-easyremote-cli-e2e.sh --skip-build --project easynet-easyremote-two-node-codex`

Evidence reports:

- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/target/e2e/docker-two-node-easyremote-cli/20260719-164635/report.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/target/e2e/host-media-device/skip-check-2/report.md`

Observed closure:

- canonical runtime convergence V2 gate is green;
- negative gate self-test is green;
- SDK cutover readiness is green, including product smokes and live daemon
  runtime events;
- downstream SDK consumer cutover is green;
- two-node Docker product evidence is green and records provider-hosted user
  Agent lifecycle/gap assertions.
