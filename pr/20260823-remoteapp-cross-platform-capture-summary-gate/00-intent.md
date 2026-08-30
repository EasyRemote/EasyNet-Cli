# Intent

Goal: prevent RemoteApp product completion from accepting cross-platform capture reports that only expose platform-level `passed_targets`. Product completion must require per-platform/per-target capture summaries for display, window, and application across macOS, Windows, and Linux.

Non-goals:
- Do not claim RemoteApp product completion.
- Do not implement or alter native OS capture backends in this dirty checkout.
- Do not duplicate the full raw capture evidence validator inside the product aggregate.

Acceptance criteria:
- `remoteapp-cross-platform-capture-e2e.sh` passed reports include compact per-target capture summaries.
- `remoteapp-product-completion-e2e.sh` validates those summaries for every required platform and target.
- Product-completion self-test rejects missing capture scenario summaries.
- Closure audit protects the aggregate requirement.
