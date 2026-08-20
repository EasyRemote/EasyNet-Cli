# Backend Boundary Self-Test Hygiene Verification

## Commands

- `bash tools/scripts/check-backend-sdk-only-boundary.sh --self-test`
- `bash tools/scripts/check-sdk-cutover-readiness.sh`
- `git diff --check`

## Expected Evidence

- The self-test still fails the forbidden backend fixture and observes all
  expected violation classes.
- Aggregate SDK cutover readiness continues to include and pass the backend
  SDK-only boundary gate.
