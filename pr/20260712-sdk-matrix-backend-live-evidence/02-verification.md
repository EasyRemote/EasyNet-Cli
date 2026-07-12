Completed checks:

- `bash tools/scripts/check-sdk-parity-matrix.sh`
- `bash tools/scripts/check-sdk-completion-audit.sh`
- `bash tools/scripts/check-sdk-cutover-readiness.sh --self-test`

The completion audit requires the developer toolchain on `PATH`; the passing
run used:

```bash
PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" \
PYTHON_BIN="/Users/macbook.silan.tech/Documents/GitHub/EasyRemote/.venv/bin/python" \
bash tools/scripts/check-sdk-completion-audit.sh
```

It passed through Python SDK live smoke, Go SDK live smoke and the Backend live
PrincipalLifecycle E2E gate.
