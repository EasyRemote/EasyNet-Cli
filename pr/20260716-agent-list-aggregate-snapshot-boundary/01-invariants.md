# Agent List Aggregate Snapshot Invariants

## Semantic Invariants

- `agent.list` remains a Device-owned read ability.
- Each returned row is still derived from an `AgentRegistry` entry plus optional hosted-agent URA projection.
- Missing hosted-agent URA remains represented as JSON null.

## Safety Invariants

- The snapshot owner performs no mutation.
- Disk load errors remain loud; the public handler must not silently drop hosted-agent identity failures.
- Test-only in-memory snapshots must not become production fallback paths.

## Boundedness Invariants

- The snapshot is loaded once per `agent.list` call.
- The projection remains linear in registered agent count.
- No new long-lived cache or background refresh loop is introduced.
