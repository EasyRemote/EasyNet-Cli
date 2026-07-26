# API Contract

## Public behavior

No public ability name changes:

- `terminal.create`
- `terminal.list`
- `terminal.close`
- `terminal.input`
- `terminal.read`
- `terminal.resize`
- `terminal.attach`

## Internal behavior

Diagnostics for ability argument validation use the canonical ability name. Constants mirror the canonical wire names while still delegating to `device_control::TERMINAL_*`.
