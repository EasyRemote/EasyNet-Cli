# Decisions Log

## 2026-07-05

- Selected Publication AbilityImpl enable/disable lifecycle as the next slice because the spec requires it and the current C ABI transport explicitly fails closed.
- Decided to build complete daemon system-ability carriers instead of deriving mutation semantics from `meta.list_abilities` rows.
- Kept Python API shape unchanged; the fix belongs in Rust daemon contract and C ABI projection.
- Added `AbilityImplLifecycleRequest` for runtime-backed paths because the existing narrow `AbilityImplID` cannot construct a complete Invocation tuple on its own.
- Preserved `AbilityImplID` compatibility for transports that can legally handle it directly, while C ABI runtime-backed transport now requires the complete lifecycle carrier.
