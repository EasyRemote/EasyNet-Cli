Implementation Delta
====================

Domain changes:
- Added AgentHostedIdentityStatus as the aggregate-owned read model for hosted
  identity status.
- Added AgentAggregateSnapshot::hosted_identity_status() for snapshot callers.
- Added AgentAggregateRepository::load_hosted_identity_status() for status
  surfaces that do not need registry rows.
- Centralized hosted-identity persistence loading behind
  load_hosted_identity_projection().

Caller migration:
- admin.status now reads joined state and hosted count through the aggregate
  projection.
- observe.network_health now reads host-device Agent URA fallback and hosted
  count through the aggregate projection.
- meta.describe now reads Device hosted count through the aggregate projection.
- invocation-history ledger resource derivation now reads host-device Agent URA
  through the aggregate projection.

Gate:
- Added R40_GOVERNANCE_STATUS_AGENT_AGGREGATE_FORK to prevent direct
  local-agents reads or file-shape inspection in governance status/read
  surfaces.
