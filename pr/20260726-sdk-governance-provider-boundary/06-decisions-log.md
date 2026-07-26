# Decisions Log

- Decision: reject governance reads in generic ability invocation instead of inferring providers.
  - Reason: provider inference in a generic action client would be another compatibility layer; typed providers already own the canonical provider/subject policy.
- Decision: keep provider names generic (`ability_descriptor`, `receipt_history`).
  - Reason: these are runtime capability providers, not product concepts.
