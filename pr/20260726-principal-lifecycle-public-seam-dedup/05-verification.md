# Verification

Planned:
- `go test ./sdk/go`
- Python principal tests or SDK test subset covering `principal`.
- `python3 sdk/conformance/rebuild_public_api_model.py`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `cargo fmt --check`
- `git diff --check`
