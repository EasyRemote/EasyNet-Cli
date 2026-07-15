# EasyNet Checkboard

`packaging/checkboard` is the packaging-side control board for EasyNet CLI
end-to-end verification. It does not contain generated logs. Runtime artifacts
are written under `target/e2e/checkboard/<run-id>/` so packaging files stay
small and reviewable.

## What It Covers

- E2E test inventory with a concrete command for each test path.
- Desktop runbook for Docker Desktop and Codex desktop usage.
- Detailed logging contract for stdout, stderr, exit code, duration, and
  failure tails.
- A runner that records every selected E2E test into an isolated log directory.
- Path descriptions for Rust integration tests, CLI daemon scripts, release
  scripts, and sibling EasyNet Docker harnesses.

## Quick Start

Run the non-external default set:

```bash
packaging/checkboard/run-checkboard.sh
```

Run only one test id:

```bash
packaging/checkboard/run-checkboard.sh --filter docker-three-node-cli-real-user
```

Run the expanded Docker three-node scenario set, including the strict
projection architecture probe:

```bash
packaging/checkboard/run-checkboard.sh --filter docker-three-node-cli --include-docker
```

Run Docker-tagged tests:

```bash
packaging/checkboard/run-checkboard.sh --include-docker
```

Run everything in the manifest, including sibling EasyNet scripts and manual
release flows:

```bash
packaging/checkboard/run-checkboard.sh --all --include-external --include-manual
```

The runner prints the final `report.md` path. Each test case also gets:

- `command.sh`
- `metadata.env`
- `stdout.log`
- `stderr.log`
- `exit_code.txt`
- `duration_ms.txt`
- `error.md` when the test fails

## Files

- `tests.manifest.tsv`: canonical inventory consumed by the runner.
- `run-checkboard.sh`: log-generating E2E runner.
- `TEST_PATHS.md`: detailed path descriptions and ownership notes.
- `LOGGING_SPEC.md`: generated-log layout and error capture contract.
- `DESKTOP_USAGE.md`: desktop operation notes for Docker Desktop and Codex.

## Boundary

This checkboard is a packaging and verification artifact. It must not redefine
EasyNet Invocation, daemon ownership, or Hub/device runtime behavior. Product
behavior stays in `easynet-daemon`, the EasyNet backend, and the Axon runtime
according to their existing boundaries.
