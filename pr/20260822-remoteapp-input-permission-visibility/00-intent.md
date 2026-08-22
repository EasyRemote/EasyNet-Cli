# RemoteApp input permission visibility gate

## Intent

Tighten RemoteApp product gates so permission recovery does not collapse host
input injection into Screen Recording-only messaging. The public
`remote_desktop.request_permission` contract requests host screen capture and
input injection permission while preserving host-local subject rules.

## Non-goals

- Do not claim RemoteApp input injection is product-complete.
- Do not permit target-resource subjects for permission probes.
- Do not create a frontend-only permission authority.
