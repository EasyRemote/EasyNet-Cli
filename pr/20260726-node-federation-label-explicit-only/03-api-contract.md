# API contract

## Input

`federation_label(n: &serde_json::Value)` reads `n.labels` when it is an object.

## Output

- Non-empty `labels["axon.federation.runtime_label"]` returns `Some(label)`.
- Missing/empty `runtime_label` returns `None`.
- `runtime_id` is ignored by this display projection.

## Error model

The function remains non-throwing. Invalid/missing label shapes project to
absence.
