# Three Time Scales in ARIS

Shared reference — maps ontology §11 to ARIS-specific operations.

## The Framework

Every operation in ARIS operates at one of three time scales.
Mixing them in a single CLI verb is a design error (ontology §11).

| Time Scale | Frequency | ARIS Example | CLI Verb |
|------------|-----------|-------------|----------|
| Schema/SLA bump | Discrete, human-in-loop | Publishing v2 of the review ability with a new input field | `easynet ability deploy` (new version) |
| Graph evolution | Low-frequency, semi-automatic | Reviewer learns that "seed fishing" suspicion is often correct; researcher's citation-discipline skill adds Semantic Scholar as Step 1.5 | None yet (internal to agent) |
| Per-call execution | Realtime | One review round: reviewer reads artifacts, scores, returns weaknesses | `easynet ability invoke`, `easynet mission run` |

## ARIS-specific implications

### What ARIS "skills" were conflating

A single ARIS SKILL.md file (e.g., `/auto-review-loop`) mixed all three:
- **Schema**: the skill's parameter interface (MAX_ROUNDS, DIFFICULTY, ...)
- **Graph**: accumulated reviewer memory, past review patterns
- **Execution**: one round of review → fix → re-review

In the EasyNet ontology, these are cleanly separated:
- Schema lives in `ability.json` (frozen, versioned)
- Graph lives in the agent's memory/workflow graph (evolves continuously)
- Execution lives in EAL missions (compiled, replayable)

### Why "ability update" was rejected

ARIS had no "update" concept — you edited the SKILL.md and it took effect.
The ontology rejects a single `ability update` verb because it conflates:
- Bumping the schema (adding a new parameter) — requires a new deploy
- Evolving the graph (learning from traces) — happens automatically
- Changing execution behavior — happens per-call based on graph state

These are three different frequencies. A single verb hides which one
the user intends, corrupting their mental model.
