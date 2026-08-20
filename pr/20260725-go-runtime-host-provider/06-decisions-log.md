# Decisions Log

## 2026-07-25

- Decision: remove the Go EasyNet provider package rather than alias it to the
  runtime provider package.
- Reason: the goal explicitly rejects compatibility layers that preserve legacy
  architecture.
- Decision: delete the daemon credentials identity projection helper.
- Reason: product credentials are downstream data; the SDK owns canonical
  runtime identity, not EasyNet daemon credential mapping.
- Decision: filter the Go SDK module import path from product-neutrality token
  scanning while adding an explicit retired-path check for `sdk/go/provider/easynet`.
- Reason: `provider/runtime` must compose the SDK root package, and the module
  path string is not itself a product abstraction. The architecture defect is
  the EasyNet provider package and credential adapter, both now blocked.
