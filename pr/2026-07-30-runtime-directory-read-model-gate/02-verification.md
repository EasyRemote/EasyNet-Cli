# Verification

- `cargo fmt --check`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash -n packaging/release/dev-check-local-runtime.sh`
- `tools/scripts/docker-media-bidi-e2e.sh --self-test`
- `cargo test session_contract_projection --lib`
- `cargo test dynamic_owner_projection_publication_promotes_read_model_ready_state --lib`
- Backend live pairing probe after Docker rebuild: `HUB_HTTP_URL=http://127.0.0.1:8080 bash scripts/probe-dev-pairing-contract.sh`
- Backend live runtime directory probe after Docker rebuild: `HUB_HTTP_URL=http://127.0.0.1:8080 EASYNET_CLI_BIN=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/target/debug/easynet EASYNET_RUNTIME_DIRECTORY_PROBE_DEVICE_READY_ATTEMPTS=20 bash scripts/probe-dev-runtime-directory-contract.sh`
- Product Docker media/bidi E2E: `EASYNET_E2E_PROJECT=easynet-media-bidi-live tools/scripts/docker-media-bidi-e2e.sh --skip-build`

