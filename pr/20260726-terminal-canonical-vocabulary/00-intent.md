# Intent

## Goal

Remove internal PTY-session ability vocabulary from the Terminal subsystem. The canonical public ability family is `terminal.*`; PTY is an implementation mechanism and must not remain in ability constant names, handler diagnostics, or module aliases that guide future architecture.

## Non-goals

- Do not change public wire ability names.
- Do not change the PTY execution backend.
- Do not remove terminal RPC/BIDI behavior.
- Do not alter backend websocket terminal compatibility.

## Acceptance criteria

- Ability constants use `ABILITY_TERMINAL_*`.
- Handler-facing diagnostics name `terminal.*`, not `pty_session_*`.
- Catalog aliases use `terminal_*_ability` instead of `pty_*_ability`.
- SPEC v2 gate rejects reintroduced `ABILITY_PTY_SESSION_*` constants.
