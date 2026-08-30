# Architecture

Layering:

1. Cross-device RemoteApp verifier validates raw live evidence.
2. Its report emits compact per-target `remoteapp_summary` entries.
3. Product-completion gate validates summary sufficiency across display, window, and application.

This keeps cross-device transport/runtime validation in the verifier while preventing shallow aggregate reports from satisfying product completion.
