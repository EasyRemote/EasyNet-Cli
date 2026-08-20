# Admin/Gateway Latest Output Boundary Plan

Goal: remove legacy output-field aliases from the Admin/Gateway SDK projection bridges and require canonical daemon DTO fields.

## Scope

- Go Admin/Gateway runtime projection bridge.
- Python Admin/Gateway profile bridge.
- Focused tests proving legacy camelCase/status aliases are rejected.
- Aggregate SDK completion audit.

## Non-goals

- No product-specific Admin/Gateway method names.
- No compatibility layer for old daemon output shapes.
- No change to public AdminClient request/response DTO names except stricter bridge input acceptance.
