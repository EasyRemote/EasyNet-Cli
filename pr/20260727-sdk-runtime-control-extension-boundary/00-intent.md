Goal: remove product Pages discovery state from the canonical SDK runtime environment model.

Problem:
- Runtime control discovery can contain provider-specific extension fields emitted by the EasyNet daemon.
- `pages_port` is a product HTTP surface fact, not a canonical runtime lifecycle or Invocation endpoint fact.
- The Python SDK currently republishes `pages_port` through `RuntimeControlDiscovery`; the Go SDK stores it in the parsed control discovery domain object.

Non-goals:
- Do not change the daemon `control.json` wire payload in this iteration.
- Do not break clients that attach to current daemon discovery files carrying `pages_port`.
- Do not add an EasyNet-specific SDK abstraction for Pages.

Acceptance criteria:
- SDK control discovery still accepts current daemon `control.json`.
- `pages_port` is not stored in Go canonical control discovery state.
- `pages_port` is not exposed by Python `RuntimeControlDiscovery`.
- Invalid `pages_port` values do not cause SDK runtime attach failure because the field is outside canonical SDK ownership.
- Tests prove provider extension acceptance without SDK projection.
