# Invariants

1. `session.open` contract negotiation may establish the reverse bidi transport, but it must not claim the product read model is online.
2. Dynamic owner projection publication is the transition that promotes the connection snapshot through `T11_REFETCH_READ_MODEL`.
3. The session escalation outbox is published only after the session contract is established.
4. Product diagnostics must require both session admission and directory/read-model visibility before reporting true online.
5. No fallback path may mark a device online from DB pairing state alone.

