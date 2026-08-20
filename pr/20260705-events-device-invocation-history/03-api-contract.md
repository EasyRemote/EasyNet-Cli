# API Contract

## Device Subscription

Input: `EventsSubscriptionRequest` with stream `device`, optional `device_ura`, `owner_ura`, `agent_ura`, `resume_cursor`, and heartbeat.

Output: complete Invocation carrier targeting daemon device event subscription ability.

## Invocation Subscription

Input: `EventsSubscriptionRequest` with stream `invocation`, required `invocation_id`, optional `resume_cursor`, and heartbeat.

Output: complete Invocation carrier targeting daemon invocation event subscription ability.

## Device History

Input: `EventsDeviceEventListRequest` with optional `device_ura`, bounded `limit`, and `cursor`.

Output: `DeviceEventPage` with stream `device`, item kind `device_event`, bounded items, and next cursor.

## Errors

- Missing complete tuple: `INVALID_ARGUMENT`.
- Stream/cursor mismatch: `INVALID_ARGUMENT`.
- Limit out of bounds: `INVALID_ARGUMENT`.
- Runtime failure: `ABILITY_FAILED`.
