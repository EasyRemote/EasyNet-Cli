# Intent

Goal: converge the Daemon SDK public boundary on the latest profile and DTO names required by `docs/spec/daemon-sdk-requirements-v1.md`.

Non-goals:
- Do not redesign daemon/Axon semantics.
- Do not add product-specific EasyNet or EasyRemote SDK concepts.
- Do not preserve legacy input aliases unless the active spec explicitly requires them.

Acceptance criteria:
- Public Go and Python SDK compatibility/profile surfaces expose latest request and method names only.
- Legacy alias tests are removed or inverted into latest-boundary tests.
- Existing complete Invocation, profile, and typed error behavior remains intact.
