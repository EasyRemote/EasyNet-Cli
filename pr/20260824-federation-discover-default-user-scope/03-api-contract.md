# API Contract

CLI surface:

- `easynet federation discover [--agent-ura <ura>] [--json]`
  reads as the currently paired User.
- `easynet federation discover --user-id <id>` performs an explicit
  User-scoped diagnostic.
- `easynet federation discover --operator-audit` performs an unfiltered local
  Authority read.
- `--operator-audit` conflicts with `--user-id`.

The JSON response shape remains `{ "entries": [...] }`.
