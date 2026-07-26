# Intent

Goal: remove the Go/Python authorized-session runtime-provider stubs that report signed stream/bidi submission as unavailable even though Runtime Core transports already expose stream and bidi carriers.

Non-goals:
- Change public invocation tuple fields or receipt semantics.
- Add EasyNet/EasyRemote-specific stream or bidi policy.
- Add fallback paths that downgrade signed invocations to unsigned drafts.

Acceptance criteria:
- RuntimeClient exposes signed stream and signed bidi carrier methods.
- AuthorizedRuntimeSession provider adapters use those methods instead of returning provider-unavailable.
- Go and Python stay aligned on the same capability state.
- Tests prove signed stream/bidi are provider-backed and nil-client validation remains intact.
