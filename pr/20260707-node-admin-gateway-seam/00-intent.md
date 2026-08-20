# Node Admin + Gateway Seam Intent

Add a Node/TypeScript Admin + Gateway profile seam that matches
`docs/spec/daemon-sdk-requirements-v1.md` without importing backend onboarding,
browser session, certificate, or account policy into the SDK.

## Scope

- Expose Node Admin carriers for agent list/start/stop/refresh and device
  session list.
- Expose pairing preflight/create/validate and device-session create/list/delete
  projection seams over injected transport methods.
- Project daemon-authored gateway status, agent records, lifecycle results,
  pairing tokens, device credentials, device sessions, and device admin results
  into stable DTOs.
- Declare Node for `admin_gateway/carrier_status` only with direct Node test
  evidence.

## Out Of Scope

- No backend account/session model.
- No certificate authority policy, token UX, browser route policy, or TLS
  provisioning behavior.
- No daemon process bootstrap hidden inside normal profile calls.
