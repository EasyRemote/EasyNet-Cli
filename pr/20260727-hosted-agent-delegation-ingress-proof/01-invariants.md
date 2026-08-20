# Invariants

1. Unsigned hosted-agent delegation request metadata may be consumed only after the dispatcher has classified the carrier as trusted daemon-local system ingress.
2. Public signed ingress must reject hosted-agent delegation request metadata before any handler sees it.
3. A signed hosted-agent delegation token must bind caller, callee, subject, nonce, and route ability.
4. The unsigned request metadata key must be removed before request metadata reaches an ability handler.
5. The issuer must not be callable with an untyped boolean authority flag.
