# Verification

## Completed

- Clean unpaired HOME with `target/debug/easynet status/start/invocation list`
  returns deterministic no-credentials failures and does not reproduce stale
  descriptor or authority-subject errors.
- `tools/scripts/docker-media-bidi-e2e.sh --self-test` passes after script
  contract updates.
- Clean Docker hub/provider/caller topology passed with report
  `target/e2e/docker-media-bidi/20260726-211038/report.md`.
- The report proves remote descriptor refs, stream/bidi product operations,
  preserved invocation tuples, verified single-terminal receipt chains, and
  plugin removal route rejection.

## Pending

- Canonical runtime convergence v2 gate after any code change.
