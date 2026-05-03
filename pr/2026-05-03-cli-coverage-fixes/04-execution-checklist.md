- [x] Reuse runtime republish/bootstrap logic after `device join` when a local
      runtime is already running.
- [x] Extend node resolution so `fleet.describe_node` can find a device across
      tenant boundaries and surface its advertised abilities.
- [x] Make `device show` consume abilities returned by `fleet.describe_node`.
- [x] Add `auth abilities` fallback for backend 404 on cross-hub node lookups.
- [x] Rewire `ability exec` onto `process.exec` + federation forward invoke.
- [x] Run focused tests and at least one feature-enabled cargo check/test pass.
