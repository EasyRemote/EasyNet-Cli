# Intent — RemoteApp Bidi Input E2E Telemetry

RemoteApp app/window sessions currently remain view-only until the daemon has a
focus-safe, target-scoped native input dispatcher. That is correct, but the
host E2E evidence must prove the actual diagnostic InvokeBidi input path fails
closed with the same policy and preserves frontend correlation telemetry.

This change extends the host view-only input safety harness so it opens the
public `remote_desktop.attach` Bidi ability for the created session, sends
pointer and keyboard frames with `sent_at_ms` plus `client_sequence`, and
requires `input_scope_unsupported` responses that echo the telemetry.

This does not claim full interactive input completion. It closes one required
product-chain seam: frontend-shaped input frames must reach the plugin's
public Bidi path and be rejected consistently when the session is view-only.
