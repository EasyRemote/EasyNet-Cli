# Decisions Log

## 2026-07-16

- Selected ability-health scan as the next Agent aggregate root-fork slice because it builds public catalog metadata from paired registry and hosted identity facts.
- Chose duplicate hosted LLM identity as "no health owner" for this advisory monitor. Selecting the first duplicate would preserve a corrupt arbitrary owner and weaken aggregate semantics.
