# Receipt URA construction boundary

## Goal

Enforce the SPEC rule that SDK facades must treat `ReceiptURA` fields as opaque
daemon/Axon-returned strings until RFC-007 lands.

## Invariants

- Receipt DTOs may preserve daemon/Axon-supplied `receipt_ura` values.
- Receipt clients may build `ReceiptRef` and causal refs only from explicit
  receipt URA plus hash facts.
- SDK production surfaces must not introduce builder/constructor helpers that
  claim a canonical receipt URA pattern before RFC-007.
- Tests, fixtures, and RFC agenda docs may continue to contain example receipt
  URA strings.

## Planned edits

- Add a static `check-sdk-receipt-ura-boundary.sh` guard with self-test.
- Run the guard from aggregate cutover readiness and scaffold checks.
- Keep the existing receipt conformance case as the behavioral contract.

## Verification

- `bash tools/scripts/check-sdk-receipt-ura-boundary.sh --self-test`
- `bash tools/scripts/check-sdk-receipt-ura-boundary.sh`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-completion-audit.sh`
- `git diff --check`
