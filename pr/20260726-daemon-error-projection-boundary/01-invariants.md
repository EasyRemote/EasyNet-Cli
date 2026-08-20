# Invariants

1. FFI remains an ABI adapter, not an invocation semantic classifier.
2. Daemon errors crossing into FFI must be reduced through an explicit typed projection.
3. Caller signer unavailability remains a caller identity readiness failure.
4. Descriptor owner offline remains a routing readiness failure.
5. Ordinary `Unavailable` daemon status remains daemon transport down unless the daemon projection classifies it as descriptor-owner-offline.
6. No old-data compatibility path is added.
