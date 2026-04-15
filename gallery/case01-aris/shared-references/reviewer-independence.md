# Reviewer Independence Protocol

Shared reference — applies to all ARIS missions that involve cross-agent review.

## Ontology Grounding

This protocol is a natural consequence of the visibility rules (ontology §4.2):
- Abilities are public — they have frozen signatures
- Skills are private — they are not visible across agent boundaries
- EAL (external) can only write `agent.ability(...)` — it cannot reach into skills

When a mission calls `reviewer.review(artifacts: [...])`, the reviewer
reads those artifacts independently. The researcher cannot pre-digest,
summarize, or interpret content before passing it — because the ability
signature accepts file paths, not summaries.

## What CAN be passed to the reviewer

- File paths (the reviewer reads contents independently)
- Review objective ("Evaluate publishability", "Score 1-10 on clarity")
- Structural metadata ("The paper has 8 sections")
- Venue constraints ("ICLR format, 9-page limit")

## What CANNOT be passed

- Executor's summary or paraphrase of file contents
- Executor's interpretation of results
- Executor's recommendations or conclusions
- Leading questions
- Previous review rounds' feedback (let the reviewer assess fresh)

## Why the ontology makes this structural

In ARIS (pre-EasyNet), this was a convention enforced by markdown
instructions. The executor *could* technically violate it by including
summaries in the MCP prompt. In the EasyNet ontology, the ability
signature enforces it: the review ability accepts `artifacts: List<FilePath>`,
not `summary: String`. The researcher literally cannot pass a summary
through the ability's public interface.

This is the difference between "please follow this rule" and
"the type system prevents you from breaking it."
