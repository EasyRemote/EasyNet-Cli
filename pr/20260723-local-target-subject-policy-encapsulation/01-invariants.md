# Invariants

1. Product adapters must not read a route target's default subject and submit it as if it were public tuple input.
2. Daemon-system invocation subject derivation is allowed only behind a named issuer method.
3. `LocalAbilityTarget` remains a cohesive route value object: ability URA, dispatch key, and callee URA.
4. Public ingress remains explicit-subject only.
5. Hub-owned system abilities may still use the ability URA as daemon-system subject, but only through encapsulated policy.
