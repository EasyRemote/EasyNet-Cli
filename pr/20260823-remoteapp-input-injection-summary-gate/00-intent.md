# Intent

Goal: prevent RemoteApp product completion from accepting input-injection reports that only expose platform-level `status=passed`. Product completion must require per-platform input summaries proving permission, pointer/keyboard application, latency bounds, target focus/geometry binding, stale sequence rejection, and terminal receipt visibility.

Non-goals:
- Do not claim RemoteApp product completion.
- Do not implement native OS input backends in this dirty checkout.
- Do not duplicate the full raw input evidence validator inside the aggregate gate.

Acceptance criteria:
- `remoteapp-input-injection-e2e.sh` passed reports include compact per-platform input summaries.
- `remoteapp-product-completion-e2e.sh` validates those summaries for macOS, Windows, and Linux.
- Product-completion self-test rejects missing input summaries.
- Closure audit protects the aggregate requirement.
