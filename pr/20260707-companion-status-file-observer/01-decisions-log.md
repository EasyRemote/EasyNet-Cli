# Decisions Log

## Shared Observer

macOS already had complete heartbeat freshness logic while Windows only parsed the status file as running. The root abstraction is status-file health classification, not platform supervision. A shared `CompanionStatusFileObserver` now owns that classification.

## Windows Stop

The Windows supervisor now stops by heartbeat pid when available and falls back to the declared executable image name. This makes stop provider-backed on Windows without adding product-specific lifecycle concepts to the SDK.
