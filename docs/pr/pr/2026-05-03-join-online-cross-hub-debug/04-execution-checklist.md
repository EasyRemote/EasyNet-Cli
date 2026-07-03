# Execution Checklist

- [x] Trace join-time daemon-config auto-wire and confirm stale `[daemon].hub_endpoint` failure mode.
- [x] Trace cross-hub `federation.forward_invoke` presence lookup and confirm `/agent` vs `/device` mismatch failure mode.
- [x] Verify CLI `--node` normalization tests.
- [x] Verify join auto-wire regression test.
- [x] Verify presence-backed `list_user_devices` tests.
- [x] Verify forward-invoke cross-tenant happy path test.
- [x] Confirm monitoring self-device URI path is canonical `/device/<node>`.
- [x] Update forward-invoke / session-request tests that still asserted legacy `/agent/<bare-node>` device URIs.
