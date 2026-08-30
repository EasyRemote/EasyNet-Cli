# Decisions Log

- 2026-08-23: Add per-target cross-device RemoteApp summaries to the verifier report rather than making product completion parse full raw cross-device evidence.
- 2026-08-23: Add `remoteapp-cross-device-remoteapp-e2e.sh` as a first-class closure-audit inventory item rather than relying only on the synthetic cross-device smoke gate.
- 2026-08-23: Product-completion status remains unclaimed; this change only hardens cross-device RemoteApp aggregate evidence.
