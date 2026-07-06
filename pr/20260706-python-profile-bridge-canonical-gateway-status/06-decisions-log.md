# Decisions Log

## 2026-07-06

- Chose gateway status as the slice because it contains duplicated lifecycle/readiness semantics in the Python facade.
- Kept native Rust/C ABI projection as the semantic owner; Python only validates canonical DTO shape.
- Preserved public API method names while intentionally removing raw-status compatibility from this internal bridge path.
- Recorded backend cutover as unresolved because remaining violations are in the sibling backend product repository, not in the SDK facade files changed by this slice.
