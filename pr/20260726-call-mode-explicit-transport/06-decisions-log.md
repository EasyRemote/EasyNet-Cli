Decisions log
=============

2026-07-26
----------

- Treat descriptor call mode as an explicit routing/proof fact, not as a
  defaultable enum whose missing value becomes RPC.
- Treat missing federation `callable_summary` as invalid because the payload
  cannot prove call mode or governed mode geometry without manufacturing a
  default RPC summary.
