# Decisions Log

- Decision: keep production registration unchanged.
  Rationale: `register` must continue using `reg.ledger_governance_owner()` so
  ownership is selected by the catalog authority context, not by each ability.
