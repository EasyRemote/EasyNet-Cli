# Wrappers Boundary Proof

## SDK-owned

- Stable DTO projections for file records and wrapper session records.
- Profile/kind validation and generic runtime ownership checks.
- Injected transport delegation for projection seams.
- Client lifecycle and closed-state behavior.

## Runtime-owned

- Runtime Core stream and bidi transport semantics used by wrapper sessions.
- Invocation dispatch, stream terminal semantics, cancellation, and receipt facts.
- File/session facts emitted by daemon governed abilities.

## Product-owned

- Backend HTTP routes, WebSocket upgrades, auth/session UX, storage quota, and content policy.
- Terminal/browser/remote desktop/media UI protocol adaptation.
- Account, billing, and dashboard state.

## Rejected designs

- Product-specific session managers inside Java/Swift SDKs.
- Backend route DTOs or browser-auth DTOs in wrapper records.
- SDK-local stream/bidi protocols that bypass Runtime Core.
- URI aliases or compatibility spellings.
- Legacy input aliases for wrapper kinds or owner fields.
