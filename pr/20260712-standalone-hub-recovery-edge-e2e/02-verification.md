Completed checks:

- `cargo test --features axon-pb --test hub_ura_tls_join_cli_e2e -- --nocapture`
- `bash tools/scripts/standalone-hub-recovery-e2e.sh --self-test`
- `bash tools/scripts/standalone-hub-recovery-e2e.sh`
- `bash tools/scripts/check-sdk-cutover-readiness.sh --self-test`
- `bash tools/scripts/check-sdk-completion-audit.sh`

The passing targeted E2E also verifies the test harness starts the Hub daemon
with the Cargo-built `easynet-keyring` binary through `EASYNET_KEYRING_BIN`.

The full completion audit passed with:

```bash
PATH="$HOME/go/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" \
PYTHON_BIN="/Users/macbook.silan.tech/Documents/GitHub/EasyRemote/.venv/bin/python" \
bash tools/scripts/check-sdk-completion-audit.sh
```
