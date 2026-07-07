# Runtime Dispatch Latest Mode Boundary Verification

## Commands

```bash
cargo test mode_omitted_is_bad_request
cargo test subject_ura_is_optional_and_trimmed
bash tools/scripts/check-daemon-latest-input-boundary.sh
bash tools/scripts/check-sdk-ura-naming.sh
git diff --check
```

## Expected Evidence

- Missing runtime-dispatch `mode` parses as a bad request.
- Explicit RPC requests still parse and preserve optional `subject_ura` behavior.
- Latest-input guard rejects legacy serde aliases and fallback-mode symbols.
