# RemoteApp permission preflight retention gate

## Intent

Gate that frontend `permission_status` remains a picker-local authorization
preflight. Denied host permissions should not bounce the user out of the share
picker before they can run `request_permission`.

## Non-goals

- Do not loosen host-local permission subject boundaries.
- Do not use frontend state as permission authority.
- Do not claim RemoteApp product completion.
