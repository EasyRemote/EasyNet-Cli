# Intent

Implement Go Admin + Gateway Runtime Core execution for daemon device-session
create/delete operations.

## Goal

- Replace Go `AdminRuntimeTransport` device-session lifecycle `NOT_IMPLEMENTED`
  seams with complete Invocation dispatch through `RuntimeClient`.
- Preserve daemon ownership of session creation, deletion, routing, and policy.
- Project daemon outputs into existing SDK `DeviceSession` and
  `DeviceAdminResult` DTOs.

## Non-Goals

- Do not export new C ABI symbols in this slice.
- Do not implement pairing/trust policy in the Go facade.
- Do not add backend/browser session identifiers to daemon session fields.
- Do not change the daemon SDK spec.
