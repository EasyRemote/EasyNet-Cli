# Verification

Passed:

```text
cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/go
go test ./...
```

Result:

```text
ok  	easynet.run/cli/sdk/go
```

Passed:

```text
cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python
python -m pytest
```

Result:

```text
484 passed
```

Passed:

```text
cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli
bash tools/scripts/check-sdk-scaffold.sh
tools/scripts/check-sdk-parity-matrix.sh
git diff --check
```

Notes:
- Direct script execution for `tools/scripts/check-sdk-scaffold.sh` is not executable on this checkout, so verification used `bash`.
- `docs/spec/daemon-sdk-requirements-v1.md` was not changed.
