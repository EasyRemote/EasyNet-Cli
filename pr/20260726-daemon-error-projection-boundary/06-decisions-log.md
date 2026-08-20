# Decisions Log

2026-07-26:
- Chose a narrow daemon-owned projection slice because the immediate architectural defect is FFI string classification, not the SDK public API.
- Kept public error payloads stable while removing FFI ownership of daemon message semantics.
- Updated convergence gates to check for the daemon-owned projection and reject the retired FFI message-classifier helpers.
