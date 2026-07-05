# Intent

Implement the missing Events profile C ABI/Rust contract backing for device subscriptions, invocation subscriptions, and bounded device event history pages.

## Goal

- Replace Python C ABI fail-closed paths for Events device/invocation streams and device history with daemon-owned carrier/projection functions.
- Keep Event stream lifecycle and runtime dispatch owned by Runtime Core.
- Keep backend fanout, GUI notifications, and product filtering outside the daemon SDK.

## Non-Goals

- Do not implement a Python event bus.
- Do not invent daemon event persistence outside governed system abilities/read models.
- Do not reinterpret Axon Invocation lifecycle events or receipt semantics in the language facade.
- Do not change the daemon SDK spec.

## Acceptance Criteria

- Rust Events contract builds complete Invocation carriers for device and invocation event subscriptions.
- Rust Events contract builds and projects bounded device event history queries.
- C ABI exports the new Events carrier/projection functions.
- Go C ABI transport binds and exercises the same Events carrier/projection
  functions instead of reporting them as unimplemented.
- Python C ABI Events transport uses Runtime Core for device/invocation subscriptions and device history.
- SDK conformance fixtures cover device subscription, invocation subscription,
  and bounded device history projection.
- Existing directory/session event behavior remains unchanged.
