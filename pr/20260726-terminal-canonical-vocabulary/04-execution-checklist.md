# Execution Checklist

- [x] Identify PTY-session ability vocabulary with codegraph/rg.
- [x] Sync codegraph index and confirm old `ABILITY_PTY_SESSION_ATTACH` symbol is absent.
- [x] Rename lifecycle/io/attach ability constants.
- [x] Rename catalog/test module aliases from `pty_*_ability` to `terminal_*_ability`.
- [x] Update handler diagnostics from `pty_session_attach` to `terminal.attach`.
- [x] Add SPEC v2 vocabulary guard and self-test fixture.
- [x] Connect terminal lifecycle args guard to the main SPEC v2 path.
- [x] Run targeted terminal tests, fmt, and gates.
- [x] Commit with required author if stable.
