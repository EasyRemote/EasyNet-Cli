# Intent

Goal: make the RemoteApp product-completion aggregate reject crash/restart recovery reports that only expose boolean coverage and a passing evidence artifact, but do not expose enough scenario summary to prove product-relevant recovery behavior at the aggregate boundary.

Non-goals:
- Do not claim RemoteApp product completion.
- Do not implement the live crash runner.
- Do not modify daemon invocation or RemoteApp runtime Rust code in this dirty checkout.

Acceptance criteria:
- `remoteapp-crash-restart-recovery-e2e.sh` still owns deep evidence validation.
- Its passed report exposes minimal recovery scenario summaries.
- `remoteapp-product-completion-e2e.sh` requires and validates those summaries.
- Self-tests reject a crash/restart report with missing scenario summaries.
