Public behavior:
- Runtime attach/connect behavior is unchanged for current daemon discovery files.
- Unknown control discovery fields still fail.
- Public Python `RuntimeControlDiscovery` no longer includes `pages_port`.
- Go has no exported `pages_port` control discovery field; internal domain state no longer stores it.

Compatibility boundary:
- `pages_port` remains accepted as a known provider wire extension only.
- It is not validated, stored, copied, or exposed by the SDK.

SDK contract:
- Products that need Pages state must use product-level APIs, not canonical runtime SDK discovery.
