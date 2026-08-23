# Architecture

Layering:

1. Input-injection verifier validates raw platform evidence and OS effect probes.
2. Its report emits compact per-platform input summaries.
3. Product-completion gate validates summary sufficiency across macOS, Windows, and Linux.

This keeps device-native input semantics in the verifier while preventing weak platform-only reports from satisfying product completion.
