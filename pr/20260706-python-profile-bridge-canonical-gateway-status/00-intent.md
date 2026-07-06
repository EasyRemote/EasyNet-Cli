# Intent

## Goal

Align the Python daemon profile bridge with the SDK single-runtime-model rule by making gateway status a canonical Admin + Gateway DTO boundary. The bridge must not maintain a second readiness/lifecycle state derivation for `gateway.status`; it should accept the same `admin_gateway/gateway_status` projection shape that Go validates through `GatewayStatus`.

## Non-goals

- Do not change `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not add product-specific EasyRemote or EasyNet backend semantics to the SDK.
- Do not keep raw status compatibility in the Python facade for `gateway.status`.
- Do not alter daemon system ability names or public AdminClient method names.

## Acceptance Criteria

- Python `DaemonProfileBridge.gateway_status` rejects non-canonical gateway status payloads instead of inferring readiness from `ready`, `running`, or partial raw daemon fields.
- Canonical `profile=admin_gateway` and `kind=gateway_status` payloads continue to pass through public `AdminClient.gateway_status`.
- Shared admin conformance records that gateway status projection is canonical and not facade-derived.
- Focused Python tests and Go/Python conformance assertions pass.
