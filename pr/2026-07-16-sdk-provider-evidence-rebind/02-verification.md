# Verification

Planned:

- `SDK_CONFORMANCE_LANGUAGES=go,python bash tools/scripts/check-sdk-conformance-reports.sh`
- `bash tools/scripts/check-sdk-completion-audit.sh --matrix-only`
- `git diff --check`

Completed:

- PASS: Python hash audit over all evidence entries in
  `sdk/conformance/runner/go-action-adapter-report.json` and
  `sdk/conformance/runner/python-action-adapter-report.json`
- PASS: `SDK_CONFORMANCE_LANGUAGES=go,python bash tools/scripts/check-sdk-conformance-reports.sh`
- PASS: `bash tools/scripts/check-sdk-conformance-reports.sh`
- PASS: `bash tools/scripts/check-sdk-completion-audit.sh --matrix-only`
- PASS: `git diff --check -- sdk/conformance/runner/go-action-adapter-report.json sdk/conformance/runner/python-action-adapter-report.json pr/2026-07-16-sdk-provider-evidence-rebind/00-intent.md pr/2026-07-16-sdk-provider-evidence-rebind/01-invariants.md pr/2026-07-16-sdk-provider-evidence-rebind/02-verification.md`
