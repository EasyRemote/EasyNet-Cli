# Citation Discipline

Private skill — internal to the researcher agent. Not network-visible.

## Why this is a skill, not an ability

Citation discipline is an internal behavioral constraint that governs HOW
the researcher writes, not a callable service. No external agent needs to
invoke "check citations" — it is an invariant enforced during paper-write
and review-response execution. Exposing it as an ability would violate the
encapsulation invariant (ontology §4.4): it would let callers dictate
internal quality control, which is the agent's own responsibility.

## Rules

1. **Never fabricate BibTeX.** All citations must come from verified sources.

2. **Lookup chain:**
   - Step 1: DBLP — `curl -s "https://dblp.org/search/publ/api?q=TITLE&format=json"`
     Get key → `curl -s "https://dblp.org/rec/{key}.bib"`
   - Step 2: CrossRef — `curl -sLH "Accept: application/x-bibtex" "https://doi.org/{doi}"`
   - Step 3: If both fail, mark with `% [VERIFY]` tag in the .bib file

3. **[VERIFY] protocol:** Papers marked [VERIFY] must be manually confirmed
   before submission. The paper-compile ability flags them in its output.

4. **Recency bias:** Prefer citations from the last 3 years. Include seminal
   older works only when they are genuinely foundational.

## Memory graph evolution

This skill's effectiveness is tracked through:
- Count of [VERIFY] tags that turn out to be real vs. phantom papers
- Citation accuracy in post-review feedback
- The lookup chain ordering may be refined (e.g., Semantic Scholar added
  as Step 1.5) based on success rates
