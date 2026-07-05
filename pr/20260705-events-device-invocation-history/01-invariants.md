# Invariants

- Product event subscriptions must lower to complete daemon system-ability Invocations.
- `control.sock` remains boot/status only; product event streams open through Runtime Core.
- Device event history is bounded by explicit page limits and cursor fields.
- Device/invocation stream requests must preserve caller, callee, subject, nonce, and causal context.
- Cursors must be stream-typed; device cursors cannot resume invocation streams and vice versa.
- Python may wrap EventStream/DeviceEventPage DTOs but must not decide daemon event terminal semantics.
