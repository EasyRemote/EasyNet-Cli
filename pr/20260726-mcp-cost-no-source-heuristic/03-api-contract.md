# API Contract

## MCP projection

Every projected MCP tool still contains:

- `x-easynet.cost_kind`
- `x-easynet.cost_label`

## Rules

- If descriptor metadata contains `cost_kind`, project that kind.
- If descriptor metadata contains `cost_label`, project that label.
- If `cost_label` is absent but `cost_kind` is declared, use the generic label for that declared kind.
- If `cost_kind` is absent, project `unknown` / `cost not declared`.

No product/source/exec heuristic may override missing cost metadata.
