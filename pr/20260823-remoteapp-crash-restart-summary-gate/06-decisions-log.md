# Decisions Log

- 2026-08-23: Keep deep crash semantics in the crash/restart verifier and expose only aggregate-required summary fields to the product-completion gate. This avoids duplicating the full verifier while preventing boolean coverage from masquerading as product recovery evidence.
- 2026-08-23: The change remains a completion-gate hardening step. It does not claim RemoteApp product completion because live cross-device/cross-platform runtime evidence is still required by the aggregate gate.
