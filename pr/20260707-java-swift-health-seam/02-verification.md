# Verification

## Commands

- `bash tools/scripts/check-java-sdk-seam.sh`
- `bash tools/scripts/check-swift-sdk-seam.sh`
- `bash tools/scripts/check-sdk-conformance-reports.sh`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-completion-audit.sh`
- `git diff --check`

## Expected Evidence

- Java seam tests exercise runtime health, diagnostics, malformed payloads,
  transport failures, and closed-client behavior.
- Swift seam tests exercise the same Health DTO/client states.
- Shared action-adapter reports include `health/api_vs_runtime` for Java and
  Swift.
- Aggregate SDK gates remain green while P1 provider and product cutover claims
  remain incomplete.
