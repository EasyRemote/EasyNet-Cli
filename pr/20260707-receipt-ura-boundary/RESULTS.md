# Verification results

- `bash tools/scripts/check-sdk-receipt-ura-boundary.sh --self-test` - pass
- `bash tools/scripts/check-sdk-receipt-ura-boundary.sh` - pass
- `bash tools/scripts/check-sdk-scaffold.sh` - pass
- `bash tools/scripts/check-sdk-product-smokes.sh` - fail in external backend product tests:
  `easynet-backend/internal/logic/skill TestListInstalled_HubError_DegradesToEmpty`
- `bash tools/scripts/check-sdk-completion-audit.sh` - fail for the same backend
  product-smoke failure after the new `SDK receipt URA boundary` gate passed

## Boundary proof

The new guard scans production SDK/C ABI surfaces for receipt-URA builder or
constructor identifiers. Opaque `receipt_ura` projection fields remain allowed;
SDK-local canonical receipt URA construction remains blocked until RFC-007 lands.
