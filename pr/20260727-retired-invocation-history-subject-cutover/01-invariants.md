Semantic invariants:
- `runtime-state/read` is the only user-owned Resource subject for invocation-history read projections.
- `session/invocation_history` is a retired carrier, including request-scoped suffixes minted as `session/invocation_history:*`.
- Retired carriers are never normalized or rewritten into live subjects.

Safety invariants:
- All-zero principal placeholders remain invalid identity facts.
- Authority validation must reject obsolete carriers before subject admission can report a misleading mismatch.
- Session authority may admit only exact user subjects or non-retired, single-segment session resource subjects.

Boundedness invariants:
- No descriptor, signer, route, or network lookup is introduced for this classification.
- Classification remains a pure core identity/read-model function.
