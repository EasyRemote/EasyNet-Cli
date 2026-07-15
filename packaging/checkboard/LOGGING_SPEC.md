# Checkboard Logging Spec

Generated logs live under:

```text
target/e2e/checkboard/<run-id>/
```

The runner never writes generated logs into `packaging/checkboard`.

## Run-Level Files

- `report.md`: human-readable result summary.
- `summary.tsv`: machine-readable case summary.
- `run.env`: run metadata such as repository root, manifest path, and start
  time.

## Case-Level Files

Each case is written to:

```text
target/e2e/checkboard/<run-id>/<case-id>/
```

Files:

- `command.sh`: exact command executed from the repository root.
- `metadata.env`: id, kind, path, tags, and description.
- `stdout.log`: full stdout.
- `stderr.log`: full stderr.
- `exit_code.txt`: process exit code.
- `duration_ms.txt`: wall-clock duration in milliseconds.
- `error.md`: generated only on failure.

## Failure Detail Contract

`error.md` must include:

- test id
- source path
- command
- exit code
- duration
- last 120 lines of stderr
- last 80 lines of stdout

This is intentionally redundant. A failed E2E should be diagnosable from the
case directory even if the terminal scrollback is gone.

## Skip Contract

Skipped rows are recorded in `summary.tsv` with status `SKIP`. Skips are used
for:

- destructive operations such as device removal
- long-running servers such as `mcp serve`
- tests requiring external repositories unless `--include-external` is passed
- manual release flows unless `--include-manual` is passed
- Docker tests unless `--include-docker` or `--all` is passed

## Exit Code

The runner exits `1` when at least one selected test fails. Skipped tests do not
make the run fail.
