# Intent

Close the `meta.forget` runtime-sync root fork where governance code loaded an
Agent aggregate snapshot and then inspected `snapshot.registry.agents`
directly.

The concrete use case is forgetting an imported descriptor for a learner Agent:
the workflow needs an optional runtime entry for hot-registration convergence
and the learner workspace manifest path for artifact removal. Those are Agent
aggregate projections, not governance workflow ownership of registry internals.
