# Intent

Converge local host device URA read projections onto the Agent aggregate
hosted-identity owner.

The current fork is read-only but still architectural: daemon local invocation
identity and clipboard context tracking both open `local-agents.json` directly
to recover the host device URA. That preserves file-shape knowledge outside the
Agent aggregate and makes future hosted-identity migration harder.

Public behavior stays stable:

- local invocation still falls back to credentials and then the unpaired local
  device URA when no valid persisted Device URA exists;
- clipboard tracking still records the persisted host device URA when joined
  and an empty device field before join;
- no lifecycle writer or profile bootstrap behavior changes in this slice.
