# Intent

Goal: make RemoteApp product completion reject lifecycle reports that only expose `status=passed`, `target_kind`, and a passing evidence artifact. Timeout, cancel, permission revoke, and resume must expose compact lifecycle summaries that the aggregate gate can validate.

Non-goals:
- Do not claim RemoteApp product completion.
- Do not change RemoteApp daemon/runtime behavior in this dirty checkout.
- Do not duplicate each host lifecycle verifier's full raw evidence validation inside the aggregate gate.

Acceptance criteria:
- Host lifecycle verifiers emit lifecycle summary fields in their passed reports.
- Product-completion aggregation requires those summaries for window and application lifecycle reports.
- Self-tests reject missing lifecycle summaries.
- Focused verifier, product-completion, and closure-audit tests pass.
