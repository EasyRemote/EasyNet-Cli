# Hot Agent Authority Aggregate Snapshot Invariants

## Semantic Invariants

- Enrollment requires a durable registry row for the hosted Agent.
- Enrollment requires exactly one `llm` hosted-agent identity row with a URA matching the canonical device realm and agent id.
- Enrollment requires hosted identity signing authority to match the canonical Device authority.
- Revocation after durable removal must fail while either the registry row or hosted identity row remains.

## Safety Invariants

- Registry-read failures remain distinguishable from identity-read failures.
- Ambiguous hosted identity rows still fail closed during enrollment.
- No declared/static authority root may override persisted hosted-agent identity.

## Boundedness Invariants

- The aggregate snapshot is loaded once per enrollment/revocation check.
- The hosted identity lookup remains linear in the hosted-agent identity file.
- No new cache, retry loop, or background refresh path is introduced.
