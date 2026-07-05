# Python Receipt Neutrality

## Objective

Remove EasyRemote-specific public naming from the Python Receipt profile while preserving daemon SDK receipt summary parsing, lifecycle state projection, and continuity guardrails required by `docs/spec/daemon-sdk-requirements-v1.md`.

## Boundary Proof

- Ownership: Receipt summary parsing and chain continuity checks belong to the daemon SDK Receipt profile, not an EasyRemote product facade.
- State: invocation lifecycle state remains the Axon-numbered SDK projection; no new terminal-state taxonomy is introduced.
- Verification: summary-only receipts still fail closed for cryptographic verification by requiring full receipt evidence.
- Compatibility posture: legacy product-named public symbols are removed rather than aliased so the SDK exposes one receipt model.

## Implementation

- Rename `EasyRemoteInvocationState` to `InvocationLifecycleState`.
- Rename `EasyRemoteReceipt` to `LocalReceiptSummary`.
- Rename `EasyRemoteReceiptChain` to `LocalReceiptSummaryChain`.
- Move receipt summary errors from `easyremote_receipt` to the canonical `receipt` profile/stage.
- Update exports, tests, and SDK docs.

## Verification

- Python Receipt tests.
- Python SDK test suite.
- Go SDK tests.
- SDK scaffold gate.
- Formatting, diff, and terminology scans.
