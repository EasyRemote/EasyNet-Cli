# Agent List Aggregate Snapshot Decisions Log

## 2026-07-16

- Decision: migrate one public read surface before attempting a broad AgentRegistry reader migration.
- Reason: CodeGraph shows dozens of readers. `agent.list` is a small public surface that already combines registry rows with hosted-agent URA state, so it is the cleanest first proof of the aggregate snapshot boundary.
- Decision: keep mutation ownership unchanged.
- Reason: lifecycle writes are already protected by `AgentLifecycleProjectionStore`; this slice targets read-side source-of-truth convergence.
