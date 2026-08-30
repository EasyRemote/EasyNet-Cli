# Invariants

1. A Unix-domain-socket manifest transport is implemented only on Unix.
2. Non-Unix builds fail closed at the same executor API instead of importing Unix-only Tokio types.
3. Open-file-handle Agent purge identity checks compile only where such handles are created.
4. Windows build closure does not claim Windows RemoteApp live-host certification.
