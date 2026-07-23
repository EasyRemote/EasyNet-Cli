# Architecture

## Owner boundary

`RuntimeReceipt` owns receipt summary projection and lifecycle state binding. Proof-fact semantics are owned by the Node runtime receipt proof-facts validator.

## Cross-language parity

The Node implementation must match:

- Rust/Go `authority_proof_expected_hash`
- Go/Python `TryNewReceiptProofFacts` / `ReceiptProofFacts`
- Java `RuntimeReceiptProofFacts`

## Layering

SDK runtime receipt validation is generic runtime behavior. It must not reference EasyNet, EasyRemote, browser, device UI, hub UI, or product directory concepts.
