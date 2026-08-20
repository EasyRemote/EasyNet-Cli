# Intent

## Goal

Remove the FFI invocation handle's case-insensitive terminal-state compatibility parser. The runtime ABI should project only the canonical public terminal strings it emits: `Completed`, `Failed`, `TimedOut`, and `Cancelled`.

## Non-goals

- Do not rename public terminal-state strings.
- Do not change Go/Python/Java/Node SDK public result models.
- Do not change receipt-chain verification.
- Do not alter daemon invocation lifecycle semantics.

## Acceptance criteria

- Canonical terminal states still project to the correct handle phases.
- Non-canonical capitalization is rejected instead of normalized.
- Focused tests cover canonical acceptance and retired case variants.
- Formatting, convergence gates, and codegraph status pass.
