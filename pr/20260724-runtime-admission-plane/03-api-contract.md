# API Contract

## Public Behavior

No public API or wire shape changes.

## Internal Contract

- Construction accepts the same `AdmissionFacade` value, but immediately wraps it in `RuntimeAdmissionPlane`.
- Tests and internals must use `admission_plane` semantics instead of raw field access.
- Transport-boundary updates return a new plane state, preserving the immutable `AdmissionFacade::with_transport_boundary` model.

## Error Contract

No error-code changes. Existing admission errors continue to originate from `AdmissionFacade`.
