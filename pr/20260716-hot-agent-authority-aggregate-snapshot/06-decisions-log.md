# Hot Agent Authority Aggregate Snapshot Decisions Log

## 2026-07-16

- Decision: migrate authority proof reads before more cosmetic Agent read surfaces.
- Reason: authority inventory is part of the proof chain. It currently reads the same paired durable state as `agent.list`, but with higher security impact.
- Decision: keep authority-domain error variants unchanged.
- Reason: callers already reason about registry unreadable, identity unreadable, missing, ambiguous, and invalid states separately. The aggregate repository should preserve source classification, not flatten errors.
