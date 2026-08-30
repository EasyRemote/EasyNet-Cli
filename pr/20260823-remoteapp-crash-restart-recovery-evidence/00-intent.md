# RemoteApp crash/restart recovery evidence gate intent

RemoteApp product readiness requires deterministic recovery after daemon,
plugin worker, control socket, or receipt-commit interruption. Existing
timeout/cancel/revoke/resume harnesses prove important lifecycle edges, but
they do not prove crash/restart recovery.

This batch adds a runner-agnostic verifier for live crash/restart artifacts. It
keeps the architecture boundary intact:

- session lifecycle remains visible through public `remote_desktop.*`
  Ability invocation;
- selected Resource URA remains the Invocation subject;
- daemon/plugin restart is recovery of product runtime state, not a new hidden
  invocation model;
- terminal receipts remain deterministic and visible after restart.

Self-test evidence only proves the verifier contract. It is not product
readiness evidence.
