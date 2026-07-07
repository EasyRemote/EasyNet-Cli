# Boundary Proof

## Root Abstraction

Admin/Gateway SDK DTOs are canonical runtime profile projections. The bridge from daemon ability output to SDK DTOs may construct missing product-neutral defaults from current request context, but it must not accept alternate field names such as `sessionId`, `deviceUra`, `expiresUnixMs`, or `status`.

## Latest-Only Rule

The canonical Admin/Gateway field names are snake_case SDK/runtime fields such as `session_id`, `device_ura`, `hub_ura`, `state`, `session_kind`, `created_unix_ms`, and `expires_unix_ms`.

Legacy aliases are architectural defects because they let downstream producers keep old shapes while the SDK claims ownership of the canonical model.

## Convergence Decision

Go and Python bridges will reject obsolete alias-only payloads by reading only canonical fields. This aligns both SDKs and prevents language-specific compatibility behavior.
