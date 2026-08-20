# Decisions Log

- Treat tracked Python bytecode as a canonical SDK ownership defect, not as a
  formatting cleanup.
- Bind Python SDK environment tests to `EXPECTED_ABI_VERSION` instead of a
  stale magic number so C ABI cutover evidence is centralized.
