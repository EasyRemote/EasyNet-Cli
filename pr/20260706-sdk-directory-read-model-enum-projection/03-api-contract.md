# API Contract

## Go SDK

- `DirectoryReadModelEnumLookup`: function type for ordinal-to-name lookup.
- `NormalizeDirectoryReadModelEnum(value, lookup)`: converts daemon read-model enum values into stable strings.
- `NormalizeDirectoryNodeState(value)`: normalizes node state projection values.
- `NormalizeDirectoryTrustLevel(value)`: normalizes node trust-level projection values.

## Error and Tenant Rules

This helper layer performs no daemon calls and no tenant authorization. It is a pure projection helper. Tenant isolation remains enforced by daemon Directory read-model queries and Invocation admission.

## Wire Shapes

- `string` returns unchanged.
- integral `float64`, `int`, `int64`, unsigned integers, and JSON numeric values use lookup when known and decimal strings when unknown.
- non-integral numeric values preserve their decimal form instead of truncating into an ordinal.
- `nil` returns empty string.
- unexpected types render with `fmt.Sprint` for diagnostic visibility.
