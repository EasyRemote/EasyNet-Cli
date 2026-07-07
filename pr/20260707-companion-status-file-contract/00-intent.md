# Companion Status File Contract

## Goal

Make the daemon-side companion status-file observer enforce the SPEC companion process contract. A status file must carry schema version, package id, package version, pid, process start time, and heartbeat time before it can project as a valid live observation.

## Boundary

- The shared observer owns status-file validation and health classification.
- Platform supervisors only choose where to read and how to fall back when no status file exists.
- Companion apps own writing the heartbeat payload; they do not classify daemon-side companion state.

## Invariants

- Missing status file remains absence, allowing platform-specific fallback only where explicitly implemented.
- Malformed or incomplete status file is an observed health error and must not fall through to process-name fallback.
- `schema_version` must be `"1"`.
- `pid`, `started_at_unix_ms`, and `last_seen_unix_ms` must be unsigned integers.
- Package id and package version must match the selected companion plan.

## Verification

- Add deterministic unit tests for missing schema version, missing pid, missing process start time, and fresh valid status files.
- Run the focused companion test suite.
- Run formatting/diff and terminology audits for touched files.

## Results

- Added deterministic tests for missing schema version, missing pid, and missing process start time.
- `cargo fmt --check` passed.
- `cargo test -q daemon::plugins::companion::heartbeat` passed.
- `cargo test -q daemon::plugins::companion` passed.
