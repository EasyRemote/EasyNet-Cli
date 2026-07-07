# Companion SDK Convergence Invariants

1. The companion model remains a daemon control-plane model, not an Axon Invocation model.
2. Public DTOs expose generic companion runtime concepts: desired state, supervisor state, observed state, projected state, boot policy, stop policy, and health mode.
3. SDK transport seams own provider calls. SDK clients only validate input, closed state, and projection decoding.
4. The Go C ABI transport owns daemon handles and therefore owns companion C ABI lifecycle calls.
5. Mandatory C ABI companion symbols fail at bind time if absent.
6. Nullable contract fields such as `error` remain nullable in SDK projections.
7. New text uses URA terminology only.
