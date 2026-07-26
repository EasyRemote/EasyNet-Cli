# Decisions Log

- 2026-07-26: Selected FFI terminal-state strict projection because codegraph and `rg` showed `explicit_terminal_phase` accepted multiple capitalization variants even though the canonical public ABI already emits exact terminal strings.
- 2026-07-26: Keep public terminal strings unchanged; remove only the compatibility parser that normalized non-canonical inputs.
- 2026-07-26: Added direct unit coverage for canonical terminal states and retired capitalization variants so future FFI changes cannot reintroduce case-insensitive lifecycle parsing.
