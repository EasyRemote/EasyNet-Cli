# Verification

All commands were run on 2026-07-07.

## EasyNet-Cli

- `cd sdk/go && go test . -run 'TestCompatibilityProjectsModelsChatStreamAndFiles|TestGoMEMCExecutesSharedProfileExclusivityConformanceCase|TestGoMEMCExecutesSharedSemanticAlignmentConformanceCase' -count=1`
  - Result: pass.
- `cd sdk/go && go test ./...`
  - Result: pass.
- `TMPDIR=/tmp bash tools/scripts/check-sdk-scaffold.sh`
  - Result: pass.
- `bash tools/scripts/check-sdk-completion-audit.sh`
  - Result: pass.

## EasyNet Backend

- `go test ./internal/handler/openai ./internal/svc ./internal/sdkboundary`
  - Result: pass.
- Backend product tests inside `bash tools/scripts/check-sdk-completion-audit.sh`
  - Result: pass.

## Search Checks

- `rg -n "RetrieveFile" sdk/go tools/scripts sdk/SDK_INTERFACE_SPEC.md sdk/README.md sdk/SDK_PARITY.md sdk/CONFORMANCE_SUITE.md`
  - Result: only the scaffold `reject_literal` and proof pack mention the retired alias.
- `rg -n "RetrieveFile" EasyNet/backend/internal/handler EasyNet/backend/internal/svc EasyNet/backend/internal/sdkboundary`
  - Result: no production or boundary references.
