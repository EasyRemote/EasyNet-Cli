# Intent

## Goal

Retire the remaining "legacy generic-route transport policy facade" ownership shape inside `DaemonInvocationService` by wrapping `AdmissionFacade` in a service-local runtime admission plane.

## Non-goals

- Do not change admission policy decisions.
- Do not weaken exact-route LocalRuntime admission.
- Do not add compatibility aliases around the old field shape.
- Do not alter public Invoke wire behavior.

## Acceptance Criteria

- `DaemonInvocationService` no longer stores a raw `admission: AdmissionFacade` field.
- Service comments describe canonical runtime admission, not legacy generic-route transport.
- Internal callsites obtain the verifier through the runtime admission plane.
- Existing route, exact-route, and dispatch behavior remains green under targeted tests and convergence gates.
