Goal
====

Remove the remaining SDK-side raw caller-signer custody error projection seam.
The SDK error boundary must expose canonical runtime error facts, not daemon
keyring implementation details, even when an upstream transport returns a raw
message.

Non-goals
=========

- Do not change daemon signer custody policy.
- Do not introduce product-specific EasyNet or EasyRemote SDK concepts.
- Do not add call-site-specific message rewrites.

Acceptance criteria
===================

- Go and Python SDK error decoders canonicalize `CALLER_SIGNER_UNAVAILABLE`
  messages through a shared semantic rule.
- Raw phrases such as `keyring entry not found`, `keyring rejected request`,
  and `self-identity:` are not exposed by decoded SDK errors.
- The caller URA remains visible when the upstream message contains one.
- Existing typed error code, retry, stage, and details remain intact.
