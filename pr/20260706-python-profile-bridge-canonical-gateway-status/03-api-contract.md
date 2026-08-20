# API Contract

## Request

`AdminGatewayStatusRequest` remains unchanged:

- `require_public_listener?: bool`
- `metadata?: object`

## Response

`ProfileBridgeDispatcher.invoke_system_ability("gateway.status", ...)` must return a canonical `GatewayStatus` DTO:

- `profile: "admin_gateway"`
- `gateway_id: string`
- `ready: bool`
- `state: string`
- `process_live: bool`
- `control_ready: bool`
- `runtime_ready: bool`
- `directory_ready: bool`
- `trust_ready: bool`
- `public_listener_ready: bool`
- `listeners: array`
- `identity: object`
- `metadata: object`

## Error Behavior

Non-canonical dispatcher output raises `SDKError` with `ErrorCode.INVALID_ARGUMENT` and `stage="admin_gateway"`.

## Compatibility

Public Python methods are stable. The internal Python bridge no longer accepts raw/legacy gateway status shapes. Consumers that still produce raw daemon status must use the native/C ABI gateway-status projection seam before returning data to the bridge.
