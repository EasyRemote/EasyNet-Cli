# Decisions Log

- 2026-07-07: Current EasyRemote already imports `easynet-sdk` and delegates
  identity helpers through `_sdk_identity`; duplicating URA helpers in this
  slice would be redundant.
- 2026-07-07: Chose to strengthen the single `ConsumerBoundaryAuditor` and
  conformance manifest instead of adding a second bespoke scanner.
- 2026-07-07: Kept this as a conformance-boundary slice only; no SDK runtime API
  changed, and full Rust/Go test suites would not add coverage for the edited
  files.
