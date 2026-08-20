# API contract

## Inputs

- `--tenant <tenant>`: explicit non-empty tenant binding.
- runtime projection tenant: used only when `--tenant` is absent.

## Errors

- Blank `--tenant`: invalid argument.
- No runtime projection and no `--tenant`: fail with an instruction to start the
  runtime or pass `--tenant`.
- Runtime projection without tenant: fail with an instruction to pass `--tenant`
  or restart the runtime.

## Output

For valid input, emitted MCP server args stay source-compatible:

`["mcp", "serve", "--tenant", "<tenant>", ...]`
