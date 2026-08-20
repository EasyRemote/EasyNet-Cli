# Intent

## Goal

Make SDK live daemon invocation failures converge on one canonical terminal failure model instead of surfacing receipt-free, shape-invalid transport payloads to Go and Python SDK clients.

## Non-goals

- Do not reintroduce legacy compatibility aliases or product-specific SDK error codes.
- Do not weaken C ABI receipt validation.
- Do not make admission denial look successful.
- Do not change public SDK method names or invocation tuple shape.

## Acceptance Criteria

- Daemon/Axon admission-denied local invokes produce an SDK-consumable typed failure outcome.
- Go and Python live smoke no longer fail with `receipt-free InvokeResponse must be a typed pre-admission Failed outcome`.
- Receipt-free responses remain accepted only for typed pre-admission failed outcomes.
- SPEC v2, architecture, SDK static, and focused smoke checks pass.
