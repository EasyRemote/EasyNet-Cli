# Verification

Completed in this slice:

- `python3 ../EasyNet-Axon/scripts/checks/check_benchmark_baselines.py --baseline ../EasyNet-Axon/sdk/rust/benches/baseline-v2.json`
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
- `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path sdk/rust/provider/easynet/pluginexec/Cargo.toml`
- `PATH="$HOME/.cargo/bin:$HOME/.nvm/versions/node/v22.16.0/bin:/opt/homebrew/bin:$PATH" mvn -q -f sdk/java/pom.xml test`
- `PATH="$HOME/.cargo/bin:$PATH" cargo test plugin_template --lib`
- `PATH="$HOME/.cargo/bin:$PATH" cargo fmt --check`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/docker-media-bidi-e2e.sh --self-test`
- `bash tools/scripts/docker-media-bidi-e2e.sh --skip-build --project easynet-media-bidi-codex-173432 --out-dir target/e2e/docker-media-bidi/codex-20260719-173432`
- `bash tools/scripts/docker-media-bidi-e2e.sh --project easynet-media-bidi-codex-full-173456 --out-dir target/e2e/docker-media-bidi/codex-full-20260719-173456`
- `DOCKER_BIN=/usr/local/bin/docker bash tools/scripts/docker-media-bidi-e2e.sh --skip-build --keep --project easynet-media-bidi-codex-20260719-190347 --out-dir target/e2e/docker-media-bidi/codex-20260719-190347`
- `DOCKER_BIN=/usr/local/bin/docker bash tools/scripts/docker-media-bidi-e2e.sh --project easynet-media-bidi-codex-full-20260719-190420 --out-dir target/e2e/docker-media-bidi/codex-full-20260719-190420`
- `PATH="$HOME/.cargo/bin:$PATH" cargo test invocation_bidi_open_rejects_missing_frame_zero_before_session_entry --lib`
- `SDK_CONFORMANCE_RUN_NONCE=0879d40513b38bb77f7d9cc2259d573f8903a1e26f38c51d869233b31866e424 PATH="$HOME/.cargo/bin:$PATH" cargo run -p sdk-conformance-runner -- --language c_abi --adapter-report sdk/conformance/runner/c-abi-action-adapter-report.json --format json`
- `PATH="$HOME/.nvm/versions/node/v22.16.0/bin:$PATH" npm test --prefix sdk/node`
- `SDK_CONFORMANCE_RUN_NONCE=<issued> PATH="$HOME/.cargo/bin:$HOME/.nvm/versions/node/v22.16.0/bin:$PATH" cargo run -p sdk-conformance-runner -- --language node --adapter-report sdk/conformance/runner/node-action-adapter-report.json --format json`
- `PATH="$HOME/.cargo/bin:$HOME/.nvm/versions/node/v22.16.0/bin:/opt/homebrew/bin:$PATH" mvn -q -f sdk/java/pom.xml test`
- `SDK_CONFORMANCE_RUN_NONCE=<issued> PATH="$HOME/.cargo/bin:$HOME/.nvm/versions/node/v22.16.0/bin:/opt/homebrew/bin:$PATH" cargo run -p sdk-conformance-runner -- --language java --adapter-report sdk/conformance/runner/java-action-adapter-report.json --format json`
- `swift test --package-path sdk/swift`
- `SDK_CONFORMANCE_RUN_NONCE=<issued> CARGO_TARGET_DIR=target/codex-conformance-swift PATH="$HOME/.cargo/bin:$HOME/.nvm/versions/node/v22.16.0/bin:/opt/homebrew/bin:$PATH" cargo run -p sdk-conformance-runner -- --language swift --adapter-report sdk/conformance/runner/swift-action-adapter-report.json --format json`
- `PATH="$HOME/.cargo/bin:$PATH" cargo test rust_bidi_open_rejects_missing_frame_zero_before_session_entry --lib`
- `SDK_CONFORMANCE_RUN_NONCE=d9cd10428ec163e7fadb7b68acf80dca9ce446c4b95ee36cfb360d1a5f10622a PATH="$HOME/.cargo/bin:$PATH" cargo run -q -p sdk-conformance-runner -- --language rust --adapter-report sdk/conformance/runner/rust-action-adapter-report.json --format json > /tmp/easynet-rust-frame0.5LFzke/rust.json`
- `PATH="$HOME/.cargo/bin:$PATH" python3 sdk/conformance/sdk_matrix.py --validate-slice rust --matrix sdk/conformance/sdk-parity-matrix.json --results-dir /tmp/easynet-rust-frame0.5LFzke`

Evidence reports:

- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/target/e2e/docker-two-node-easyremote-cli/20260719-164635/report.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/target/e2e/docker-two-node-easyremote-cli/20260719-170651/report.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/target/e2e/docker-media-bidi/20260719-170938/report.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/target/e2e/docker-media-bidi/codex-20260719-173432/report.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/target/e2e/docker-media-bidi/codex-full-20260719-173456/report.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/target/e2e/docker-media-bidi/codex-20260719-190347/report.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/target/e2e/docker-media-bidi/codex-full-20260719-190420/report.md`
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
  Python, Go, Rust, Java, and Node are template-backed for declarative exec
  invoke; stream and bidi helper cells remain closed seams until their helpers
  own streaming frames.
- Rust plugin templates now use the provider-scoped
  `easynet-provider-pluginexec` helper crate and generate compiled source that
  produces `bin/exec-plugin` through `make build`; the template does not
  hand-write sidecar request parsing.
- Java plugin templates now use the provider-scoped
  `run.easynet.daemon.provider.easynet.pluginexec` helper package and the
  product-neutral `SidecarRuntime.serve` API. They generate compiled source
  plus a `bin/exec-plugin` JVM wrapper through `make build`; the template does
  not hand-write sidecar request parsing.
- Section 12 now pins EasyNet-Cli runtime/source evidence to implementation
  commit `a1b886d62e5d597505acf37c23a06b77e9afa0df`, the verified Java
  provider-helper and Docker media-bidi evidence source revision.
- C ABI now has direct executable `bidi/frame0_required` evidence at the C ABI
  boundary. It rejects missing frame-0 construction material before active bidi
  session allocation; Rust, Java, and Swift remain listed as unproven for that
  requirement.
- Node now has direct executable `bidi/frame0_required` evidence in the SDK
  runtime facade. `RuntimeClient.openBidi` rejects omitted or empty stream
  descriptors before the transport is called; Rust and Swift remain listed as
  unproven for that requirement.
- Java now has direct executable `bidi/frame0_required` evidence in the runtime
  facade. `RuntimeClient.openBidi` and `AsyncRuntimeClient.openBidiAsync`
  reject null frame-0 material as canonical `SDKError.INVALID_ARGUMENT` before
  the runtime transport is called; Rust remains listed as unproven for that
  requirement.
- Swift now has direct executable `bidi/frame0_required` evidence in the runtime
  facade. `RuntimeClient.openBidi` accepts the existing non-nil `BidiFrame`
  caller shape while rejecting nil frame-0 material as canonical
  `SDKError.invalidArgument` before the runtime transport is called.
- Rust now has direct executable `bidi/frame0_required` evidence on the internal
  Rust open path. `bidi_open_with_axon_pb` rejects missing stream/frame-0
  construction material before active bidi session allocation. The
  `bidi/frame0_required_other_languages` unproven requirement is now removed
  from the canonical public API model.
- Revision-pinned Rust RF-4 runner output: `bidi/frame0_required` passed with
  selector `rust_bidi_open_rejects_missing_frame_zero_before_session_entry`,
  EasyNet-Cli tree digest
  `31fb9c3c9065194e5c6517c41b6b42977fd87c73648849ec0086b039c42680a4`,
  Axon revision `ad060b8736f715440cbb68680790380ad29fd5f4`,
  case digest
  `bd01cbf63639c2440c3a20e794d66f295907956272911391b9b2ab12b711e7b3`,
  evidence digest
  `d845cd1b4bd9a1895a6d146fcee77ba36a4a39965c94537f2c87b2b10fb06ffa`,
  attestation
  `8336cd5d9d56f345ede68fa63117252698809bd878ef8c68a3c6149db25718e5`,
  and execution output digest
  `1ba91d7062780e6701c4153b748d7efe74199091600252d49699d4e531620199`.
- The Section 11.9 fixed-baseline benchmark acceptance item is now explicitly
  represented in Section 12 with the Axon `canonical-local-runtime-v2` baseline
  digest and the standalone benchmark-baseline validator. The baseline covers
  unary, stream, bidi, cooperative cancellation cleanup, allocation counts,
  allocated bytes, active-invocation cleanup, and bounded concurrency.
