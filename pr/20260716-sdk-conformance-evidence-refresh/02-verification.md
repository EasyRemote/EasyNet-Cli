# Verification

Passed:

```text
python3 -m py_compile sdk/conformance/refresh_adapter_report_evidence.py
python3 sdk/conformance/refresh_adapter_report_evidence.py --self-test
python3 sdk/conformance/refresh_adapter_report_evidence.py --write
python3 sdk/conformance/refresh_adapter_report_evidence.py --check
SDK_CONFORMANCE_LANGUAGES=rust,c_abi,python \
  bash tools/scripts/check-sdk-conformance-reports.sh
git diff --check
```

The runner captured one source snapshot and executed every selected binding:

- Rust: 17 passed, 22 manifest-declared unsupported.
- C ABI: 18 passed, 21 manifest-declared unsupported.
- Python: 36 passed, 3 manifest-declared unsupported.

Each result set has one `tree_sha256`, matching the generated source
attestation. Go evidence is current under `--check`; its runner execution is
not part of this slice because its parity selector requires the complete
seven-language live-result set, an external CI publication input.
