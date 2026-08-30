# RemoteApp permission preflight gate

## Intent

Gate the frontend RemoteApp permission preflight path so product UI can check
host-local permission readiness through `remote_desktop.permission_status`
before creating a session.

## Non-goals

- Do not move permission authority out of the daemon plugin.
- Do not allow target-resource subjects for permission probes.
- Do not claim product-complete OS input injection from a permission preflight.
