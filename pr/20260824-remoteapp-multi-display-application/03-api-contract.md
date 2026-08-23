# API contract

`device_capabilities.metadata.platform_support.platforms.<os>.application`
adds `application_surface`:

- `scope`: `display_scoped` or `process_scoped`
- `multi_window`: boolean
- `multi_display`: boolean
- `blocked_reason`: null or canonical target reason

This is capability projection only; it does not bypass target binding,
capture proof, rebind, receipt, or terminal lifecycle contracts.
