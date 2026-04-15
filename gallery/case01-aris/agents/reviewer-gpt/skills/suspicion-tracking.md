# Suspicion Tracking

Private skill — internal to the reviewer agent. Not network-visible.

## Why this is a skill, not an ability

Suspicion tracking is the reviewer's internal memory of what to watch for
across review rounds. If this were an ability, the researcher (executor)
could read the reviewer's suspicions and preemptively address only the
tracked items while hiding other issues. The whole point of adversarial
review is that the reviewer's internal state is opaque to the executor.

This is a textbook demonstration of the encapsulation invariant
(ontology §4.4): "No CLI command, no SDK call, and no EAL construct may
reach across an agent boundary into a skill."

## How it works

In ARIS, this was REVIEWER_MEMORY.md — a file that the reviewer wrote
and read back, but that the executor (Claude) could technically access.
In the EasyNet ontology, this becomes part of the reviewer agent's
private memory graph, which the executor cannot access at all.

### Tracked signals

Per round, the reviewer records:
- **Suspicions**: claims that feel too good, patterns that suggest p-hacking
- **Unresolved concerns**: weaknesses not yet addressed
- **Behavioral patterns**: does the executor only address the easiest
  weaknesses? Does it reframe instead of fixing?
- **Verified vs. unverified claims**: which results the reviewer
  independently confirmed

### Evolution

The suspicion tracking strategy itself evolves:
- Which suspicion patterns most often predict actual problems?
- Which types of rebuttals are genuine vs. deflection?
- Calibration: is the reviewer too strict or too lenient at each score level?
