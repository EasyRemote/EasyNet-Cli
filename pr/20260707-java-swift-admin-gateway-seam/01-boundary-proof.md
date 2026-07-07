# Admin + Gateway Boundary Proof

## SDK-owned

- Complete Admin carrier request DTOs with caller, callee, subject, descriptor version, nonce, causal context, and metadata.
- Projection DTOs for daemon gateway status, agent records, lifecycle results, pairing facts, device credentials, and daemon device sessions.
- Client lifecycle and injected transport delegation.
- Validation that opaque daemon identifiers are not path-like and that hosted-agent lifecycle does not manage device system agents.

## Daemon-owned

- Gateway process lifecycle and public listener state.
- TLS/trust readiness, directory/runtime readiness, and degraded readiness facts.
- Agent lifecycle execution, pairing validation, credential creation/verification, and device-session lifecycle.
- System ability dispatch and receipt generation.

## Product-owned

- EasyRemote `Server`/`AgentControl` ergonomics and Python process hosting.
- Backend account tables, browser sessions, OAuth/JWT policy, onboarding copy, and certificate provisioning UX.
- Hub dashboard route shape and public HTTP rendering.

## Rejected designs

- Backend/browser session DTOs in SDK device-session records.
- EasyRemote-specific lifecycle classes in Java/Swift facades.
- SDK-local derivation of aggregate gateway readiness from partial daemon flags.
- Ability+args-only Admin carriers without complete Invocation tuple context.
- URI aliases or compatibility spellings.
