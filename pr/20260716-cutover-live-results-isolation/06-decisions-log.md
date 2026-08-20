# Decisions Log

## 2026-07-16

- Selected the live-result isolation slice because Section 20 of the audit
  requires release scripts to consume an explicit results directory generated
  after source edits settle.
- Kept `check-sdk-conformance-reports.sh` defaults unchanged so manual
  language-slice workflows keep current behavior.
- Chose a cutover-script self-test over committed generated artifacts because
  live JSON records contain source-tree and run-nonce attestations that are
  build evidence, not source truth.
