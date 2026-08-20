# Mission Child Target Agent Aggregate Decisions Log

## 2026-07-16

- Decision: migrate Mission child-target reads before display/catalog readers.
- Reason: Mission execution creates child Invocations; target proof belongs in
  the proof chain, not in display metadata.
- Decision: keep Mission error wording specific.
- Reason: callers already rely on actionable messages for EAL target collision
  and hosted Agent missing/invalid cases. The aggregate owner should centralize
  facts, not flatten errors.
