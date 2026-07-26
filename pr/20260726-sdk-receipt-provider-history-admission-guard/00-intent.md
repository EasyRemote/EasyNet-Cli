# Intent

## Goal

Make the public SDK receipt-history provider enforce the same canonical history admission guard as `AuthorizedRuntimeSession.History()`.

## Non-goals

- Do not add a product-specific EasyNet/EasyRemote receipt model.
- Do not change the public receipt provider API shape.
- Do not weaken session/delegation authority validation.

## Acceptance criteria

- Go `RuntimeReceiptProvider.List` rejects non runtime-state read subjects before descriptor resolution.
- Python `RuntimeReceiptProvider.list` rejects non runtime-state read subjects before descriptor resolution.
- Python history validation is shared instead of duplicated between the session wrapper and receipt provider.
- Existing authorized session history behavior remains unchanged.
- SDK conformance and runtime convergence gates pass.
