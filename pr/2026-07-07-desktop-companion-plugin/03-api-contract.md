# API Contract

## Manifest

- `kind = "desktop_companion"` enables the companion schema.
- `[companion]` is required and validates lifecycle, boot policy, stop policy, health, and platform sections.
- Declared executable artifacts must live under `bin/` or `dist/` so the package hash covers them.

## CLI and Control

- `easynet plugin list` includes companion columns and JSON companion DTOs.
- `easynet plugin status <id> --json` returns `DesktopCompanionStatus`.
- `easynet plugin enable|disable <id>` returns `DesktopCompanionActionResult`.
- Daemon-local `plugin.companion_status` and `plugin.companion_reconcile` return the same DTO contract and remain local-only.

## SDK/FFI

- `easynet_companion_*` C ABI functions return the shared JSON contract.
- Go, Python, Swift, and Java SDK companion APIs parse the shared status/action DTOs.
- Axon SDK surfaces do not expose companion lifecycle.

## Error Semantics

Supervisor install failure fails plugin install. Post-Ready start failure is recorded as status and warning, not daemon boot failure.
