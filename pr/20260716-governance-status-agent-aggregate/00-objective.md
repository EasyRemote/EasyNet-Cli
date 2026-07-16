Objective
=========

Converge governance status/read handlers onto the Agent aggregate owner for
hosted-Agent identity facts.

Public behavior stays stable: admin.status, observe.network_health,
meta.describe, and invocation-history ledger resource derivation continue to
return the same fields and tolerate the same hosted-identity absence. The
internal source of truth changes from direct LocalAgentsFile inspection to a
typed aggregate projection.
