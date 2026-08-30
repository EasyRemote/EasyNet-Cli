# Invariants

- `ability invoke` receives a full `easynet:///r/.../ability/...` URA, not
  `remote_desktop.*` short names.
- The harness resolves lifecycle abilities from `ability list --format json`
  and requires exactly one rpc descriptor for each public ability.
- Catalog resolution and session approval causal-context projection are shared
  helper behavior, not duplicated per harness.
- The fix stays in the harness/product-proof layer. It does not add CLI
  fallback parsing or weaken Invocation boundary validation.
- Session lifecycle proof remains bounded: timeout, cancel, and resume harnesses
  must still validate deterministic terminal/session state through public
  session views.
- Session lifecycle calls after `create_session` must use the session consent
  approval receipt as scalar causal context. Root causal context is only valid
  for root calls, not for session-bound lifecycle calls.
- The RemoteApp product-complete matrix must remain false until real
  cross-platform capture, input, media, network, recovery, frontend, and
  cross-device evidence exists.
