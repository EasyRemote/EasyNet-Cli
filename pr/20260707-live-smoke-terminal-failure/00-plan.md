# Live Smoke Typed Terminal Failure Plan

## Goal

Close the SPEC section 30 live daemon smoke gap by proving the Go and Python SDK live smokes cover unary, stream, bidi file transfer, and typed terminal failure through the canonical SDK runtime model.

## Scope

- Keep the public Go and Python SDK APIs unchanged.
- Add typed terminal failure assertions to the existing live daemon smoke flows.
- Tighten smoke self-tests so future regressions cannot omit file transfer or terminal failure coverage.
- Avoid product-specific SDK abstractions, directory models, receipt models, or lifecycle aliases.

## Verification

- `bash tools/scripts/go-sdk-live-smoke.sh --self-test`
- `bash tools/scripts/python-sdk-live-smoke.sh --self-test`
- `bash tools/scripts/go-sdk-live-smoke.sh`
- `bash tools/scripts/python-sdk-live-smoke.sh`
- `bash tools/scripts/check-sdk-cutover-readiness.sh --self-test`
- `bash tools/scripts/check-sdk-completion-audit.sh`
- `git diff --check`
