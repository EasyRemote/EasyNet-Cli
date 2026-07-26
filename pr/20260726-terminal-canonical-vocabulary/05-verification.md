# Verification

Completed checks:

- `codegraph sync .` — indexed 15 changed files.
- `codegraph query ABILITY_TERMINAL_ATTACH --path .` — canonical symbol present at the terminal attach source and catalog/test imports.
- `codegraph query ABILITY_PTY_SESSION_ATTACH --path .` — no results after sync.
- `rg -n "ABILITY_PTY_SESSION|pty_session_(create|list|close|input|read|resize|attach)|pty_lifecycle_ability|pty_io_ability|pty_attach_ability" src/daemon/ability src/daemon/invocation tests tools --glob '!target/**'` — retired tokens only appear inside the new gate forbidden-token list and self-test fixture.
- `cargo test daemon::ability::builtins::device_control::terminal --lib` — 48 passed.
- `cargo test real_device_terminal --lib` — 6 passed.
- `cargo fmt --check` — passed after `cargo fmt`.
- `git diff --check` — passed.
- `bash tools/scripts/check-architecture-convergence.sh` — `architecture-convergence: OK`.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — `canonical-runtime-convergence-v2: OK`.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test` — `canonical-runtime-convergence-v2 self-test ok`.
