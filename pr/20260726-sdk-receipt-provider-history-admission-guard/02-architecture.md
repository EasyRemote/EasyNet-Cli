# Architecture

## Boundary

`RuntimeReceiptProvider` is the canonical SDK receipt read-model provider. It must not rely on higher-level product/session wrappers to enforce receipt-history admission.

## Go

Reuse the existing same-package `validateSessionHistoryRequest` guard from `RuntimeReceiptProvider.List`.

## Python

Move session-history validation into `_receipt_history_admission.py` and import it from both `authorized_runtime_session.py` and `receipt.py`.

## Result

Products using `RuntimeReceiptProvider` directly get the same fail-closed behavior as products using `AuthorizedRuntimeSession.History()`.
