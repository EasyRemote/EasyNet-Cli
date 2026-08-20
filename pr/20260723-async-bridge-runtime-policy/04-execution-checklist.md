# Execution Checklist

- [x] Rename `NoRuntimeFallback` to `SyncBridgeRuntimePolicy`.
- [x] Rename `fallback` parameters and comments to policy/state language.
- [x] Migrate MCP, device ability management, LocalRuntime invoker, and smoke
      binary call sites.
- [x] Rename tests that encode fallback vocabulary.
- [x] Add SPEC v2 regression coverage for the retired policy name.
- [x] Update design/RFC documentation that still referenced the retired type.
- [x] Run formatting, targeted tests, and convergence gates.
