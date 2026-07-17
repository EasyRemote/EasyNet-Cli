# Intent

Implement the V2 `restart_recover` lifecycle seam for the canonical Go and Python SDK runtime model.

The change is intentionally provider-backed rather than daemon-specific: SDKs declare the bounded recovery request, provider delegation point, and recovery report proof. Runtime providers own orphan scans, terminal fact replay, and cleanup before returning `runtime_started`.
