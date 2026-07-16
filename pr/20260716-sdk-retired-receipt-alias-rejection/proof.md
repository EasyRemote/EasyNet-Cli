# SDK Retired Receipt Alias Rejection Proof

## Root Fork

The Runtime Core SDK projections had two states for the retired top-level
`receipt` field:

- explicit projection was already removed from Go and Python canonical models;
- silent JSON acceptance remained because the decoders ignored unknown fields.

That leaves a legacy wire shape accepted at the canonical boundary even though
the v5 provider model requires `terminal_receipt` for terminal execution facts.

## CodeGraph-Style Evidence

- `sdk/go/runtime.go:929` decodes `InvocationResult` with a local DTO. Go's
  `encoding/json` ignores unknown fields, so a top-level `receipt` field is
  accepted unless explicitly rejected.
- `sdk/go/stream.go:461` and `sdk/go/bidi.go:625` have the same DTO decode
  pattern for stream and bidi terminal event projections.
- `sdk/python/easynet_sdk/runtime.py:505`,
  `sdk/python/easynet_sdk/stream.py:60`, and
  `sdk/python/easynet_sdk/bidi.py:83` parse JSON into dictionaries and read
  only canonical fields. A top-level `receipt` field is therefore accepted and
  discarded.
- Existing Go/Python tests named the behavior as "ignores legacy
  receipt-only", proving the remaining compatibility path was intentional but
  no longer aligned with the v5 boundary.
- `tools/scripts/check-architecture-convergence.sh` already bans explicit
  `decoded.get("receipt")` and `json:"receipt"` patterns, but it did not prove
  decoders reject the alias.

## Boundary Decision

`receipt` remains a valid domain word inside receipt resources, causal context
payloads, and typed receipt objects. The retired shape is only the top-level
runtime result/frame alias that competes with `terminal_receipt`.

The fix belongs in the SDK facade decoders because they own canonical language
projection validation. No daemon protocol or C ABI symbol changes are needed.

## Invariants

- Unary invocation results must accept `terminal_receipt` and
  `admission_receipt`.
- Stream and bidi terminal projections must accept `terminal_receipt` and
  `admission_receipt`.
- Top-level `receipt` must fail with typed invalid-argument SDK errors in Go
  and Python canonical decoders.
- Payload JSON may still contain application data named `receipt`; the guard is
  only for the top-level retired alias.
- Direct runtime provider projections remain unchanged when they emit canonical
  `terminal_receipt`.

## Verification Plan

- Go focused tests:
  - `go test ./... -run 'TestInvocationResultSeparatesAdmissionAndTerminalReceipts|TestStreamEventRejectsLegacyReceiptAlias|TestStreamEventPreservesTopLevelCanonicalReceipt|TestStreamEventRejectsLegacyReceiptOnlyField|TestBidiFrameRejectsLegacyReceiptOnlyField'`
- Python focused tests:
  - `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_runtime.py sdk/python/tests/test_stream.py sdk/python/tests/test_bidi.py -k 'receipt or legacy_event_alias'`
- Architecture gates:
  - `bash tools/scripts/check-architecture-convergence.sh`
  - `bash tests/scripts/test_check_architecture_convergence.sh`
- Diff hygiene:
  - `git diff --check`

## Verification Results

- PASS: `go test ./... -run 'TestInvocationResultSeparatesAdmissionAndTerminalReceipts|TestStreamEventRejectsLegacyEventAlias|TestStreamEventPreservesTopLevelCanonicalReceipt|TestStreamEventRejectsLegacyReceiptOnlyField|TestBidiFrameRejectsLegacyReceiptOnlyField'`
- PASS: `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_runtime.py sdk/python/tests/test_stream.py sdk/python/tests/test_bidi.py -k 'receipt or legacy_event_alias'`
- PASS: `bash tools/scripts/check-architecture-convergence.sh`
- PASS: `bash tests/scripts/test_check_architecture_convergence.sh`
- PASS: `git diff --check`
