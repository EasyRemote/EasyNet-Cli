# SDK-owned EasyRemote receipt facade

## Objective

Move EasyRemote receipt-summary parsing, state projection, summary-only verification guardrails, and hash-chain continuity projection behind the EasyNet-Cli Python SDK Receipt profile.

## Boundary

- Axon and the daemon remain the source of truth for receipt bodies and cryptographic verification.
- The SDK owns the facade/projection behavior available from daemon receipt summaries.
- EasyRemote keeps its public `Receipt`, `ReceiptChain`, and error vocabulary as thin API wrappers only.

## Non-goals

- Do not fabricate receipt URAs.
- Do not claim cryptographic verification from summary-only daemon receipts.
- Do not edit `docs/spec/daemon-sdk-requirements-v1.md`.
