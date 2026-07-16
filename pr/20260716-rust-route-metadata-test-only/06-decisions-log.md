# Decisions Log

## 2026-07-16

- Chose `#[cfg(test)]` for Rust proof metadata because the constants are valid
  generator evidence but not runtime behavior.
- Kept Go/Python generated metadata unchanged because those SDK modules expose
  provider-route metadata to their own test/static contracts.
- Rejected suppressing warnings with `allow(dead_code)` because it would keep
  proof-only data in production surfaces.
