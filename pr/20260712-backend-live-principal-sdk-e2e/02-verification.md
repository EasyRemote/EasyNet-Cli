Completed checks:

- Backend tagged live E2E:
  `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" bash tools/scripts/backend-live-principal-e2e.sh`
  passed.
- CLI gate self-test:
  `bash tools/scripts/backend-live-principal-e2e.sh --self-test`
  passed.
- CLI aggregate readiness self-test:
  `bash tools/scripts/check-sdk-cutover-readiness.sh --self-test`
  passed.
- Focused downstream cutover gate:
  `bash tools/scripts/check-downstream-sdk-consumer-cutover.sh /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend /Users/macbook.silan.tech/Documents/GitHub/EasyRemote`
