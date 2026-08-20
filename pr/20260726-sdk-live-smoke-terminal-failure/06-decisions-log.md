# Decisions Log

2026-07-26:

- Use SDK live smoke failure as the next convergence slice because it hits Go and Python through the same C ABI/runtime boundary.
- Do not restore retired `ErrorCode.DAEMON_OFFLINE` or `provider/easynet` compatibility packages in the canonical SDK; downstream product failures must be cut over instead.
- Keep C ABI receipt-free failure validation strict; fix daemon admission facts so failures carry canonical code/stage/security class.
- Do not let `policy_gate` read local credentials. Pairing credentials are converted into trust-anchor owner facts at daemon boot through `RuntimeTrustContext`.
- Make FFI stream resources explicit: active reader, terminal-drained, then owner-close release. Unknown ids and cross-handle closes still fail.
