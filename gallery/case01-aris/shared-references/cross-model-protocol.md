# Cross-Model Adversarial Collaboration Protocol

Shared reference — the foundational interaction pattern in ARIS.

## The Rule

Executor and reviewer must be different model families.

| Role | Agent | Backed by | Why |
|------|-------|-----------|-----|
| Executor | silan/researcher | Claude | Writes code, runs experiments, drafts papers |
| Reviewer | openai/reviewer | GPT-5.4 | Critiques, scores, demands revisions |
| Reviewer (alt) | google/reviewer | Gemini | Visual review, long-context assessment |

## Ontology Mapping

In ARIS, this was a markdown convention: "executor and reviewer must be
different model families." In the EasyNet ontology, this becomes a
structural property:

1. **Different agents** — the researcher and reviewer are separate agent
   instances with separate identity (`silan/researcher` vs `openai/reviewer`)
2. **Separate memory graphs** — each agent's accumulated knowledge is private
3. **Ability boundary** — the only communication channel is through public
   abilities, enforced by the encapsulation invariant
4. **No shared skills** — even if both agents have a skill called
   "review_calibration", they are independent copies with independent
   evolution

## Three Difficulty Levels

| Level | Cross-agent pattern | Former ARIS |
|-------|-------------------|-------------|
| Medium | researcher → reviewer.review | MCP call |
| Hard | + reviewer.rule_on_rebuttal (debate) | MCP + memory file |
| Nightmare | + reviewer.adversarial_audit (repo read) | codex exec |

## Why OOP matters here

The visibility rule (ontology §4.2) guarantees reviewer independence
structurally. The researcher cannot:
- Read the reviewer's suspicion-tracking skill
- Modify the reviewer's memory graph
- Pre-filter what the reviewer sees

These were all *possible* violations in ARIS. In the EasyNet ontology,
they are *impossible* — the type system prevents them.
