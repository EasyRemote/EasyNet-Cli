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
- `PATH="$HOME/.nvm/versions/node/v22.16.0/bin:$PATH" npm test --prefix sdk/node`
- `PATH="$HOME/.nvm/versions/node/v22.16.0/bin:$PATH" node --check sdk/node/provider/easynet/pluginexec.js`
- `PATH="$HOME/.cargo/bin:$PATH" cargo test plugin_template --lib`
- `PATH="$HOME/.cargo/bin:$PATH" cargo fmt --check`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/docker-media-bidi-e2e.sh --self-test`
- `bash tools/scripts/docker-media-bidi-e2e.sh --skip-build --project easynet-media-bidi-codex-173432 --out-dir target/e2e/docker-media-bidi/codex-20260719-173432`
- `bash tools/scripts/docker-media-bidi-e2e.sh --project easynet-media-bidi-codex-full-173456 --out-dir target/e2e/docker-media-bidi/codex-full-20260719-173456`
- `PATH="$HOME/.cargo/bin:$PATH" cargo test invocation_bidi_open_rejects_missing_frame_zero_before_session_entry --lib`
- `SDK_CONFORMANCE_RUN_NONCE=0879d40513b38bb77f7d9cc2259d573f8903a1e26f38c51d869233b31866e424 PATH="$HOME/.cargo/bin:$PATH" cargo run -p sdk-conformance-runner -- --language c_abi --adapter-report sdk/conformance/runner/c-abi-action-adapter-report.json --format json`

Evidence reports:

- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/target/e2e/docker-two-node-easyremote-cli/20260719-164635/report.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/target/e2e/docker-two-node-easyremote-cli/20260719-170651/report.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/target/e2e/docker-media-bidi/20260719-170938/report.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/target/e2e/docker-media-bidi/codex-20260719-173432/report.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/target/e2e/docker-media-bidi/codex-full-20260719-173456/report.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/target/e2e/host-media-device/skip-check-2/report.md`

Observed closure:

- canonical runtime convergence V2 gate is green;
- negative gate self-test is green;
- SDK cutover readiness is green, including product smokes and live daemon
  runtime events;
- downstream SDK consumer cutover is green;
- two-node Docker product evidence is green and records provider-hosted user
  Agent lifecycle plus caller-side canonical invocation success;
- Node plugin sidecar helper is provider-scoped and covered by SDK/helper
  tests; `plugin init --language node` generates a helper-backed template
  instead of naked sidecar frame parsing.
- Provider sidecar helper capability evidence is language/call-mode scoped:
  Python, Go, and Node are template-backed for declarative exec invoke; stream
  and bidi helper cells remain closed seams until their helpers own streaming
  frames.
- C ABI now has direct executable `bidi/frame0_required` evidence at the C ABI
  boundary. It rejects missing frame-0 construction material before active bidi
  session allocation; Rust, Node, Java, and Swift remain listed as unproven for
  that requirement.
