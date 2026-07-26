# MCP install runtime tenant cutover

## Goal

Remove the `easynet mcp install` fallback that writes `--tenant default` when
neither `--tenant` nor a runtime session projection supplies a tenant. MCP
install must bind to an explicit runtime tenant fact or fail closed.

## Non-goals

- Do not change the installed MCP server command shape for valid inputs.
- Do not remove `--tenant`; it remains the explicit operator override.
- Do not add another config source or compatibility alias.

## Acceptance criteria

1. Explicit non-empty `--tenant` is accepted.
2. Missing `--tenant` uses the tenant from `~/.easynet/runtime.json` only when
   that projection carries a non-empty tenant.
3. Missing runtime projection or missing tenant fails before writing client
   config.
4. SPEC v2 gate rejects future `"default"` tenant fallback in MCP install.
