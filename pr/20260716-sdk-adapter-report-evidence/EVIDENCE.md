# Evidence

## Source exploration

- `tools/scripts/check-sdk-scaffold.sh` required SDK artifacts did not include `sdk/conformance/refresh_adapter_report_evidence.py`.
- `tools/scripts/check-sdk-scaffold.sh` validated adapter reports only for schema version and records existence.
- `tools/scripts/check-sdk-conformance-reports.sh` validates forged hashes only through the runner self-test, but the normal gate did not preflight all committed adapter report evidence hashes before snapshot/execution.
- `tools/sdk-conformance-runner/src/main.rs` already rejects stale evidence hashes during per-language execution; the missing layer is a cheap repository-wide currentness gate for all committed reports.
- `docs/spec/daemon-sdk-requirements-v1.md` now requires SHA-256 pinned evidence sources and runner-owned executions.

## Intended convergence

The report JSON files remain coverage manifests. The refresh script becomes the single local maintenance/check tool for derived evidence SHA-256 values, and the conformance gate uses it before live execution.
