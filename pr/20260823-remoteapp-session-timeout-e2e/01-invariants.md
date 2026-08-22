# Invariants — RemoteApp Session Timeout E2E

1. The selected Resource URA must remain the Invocation subject for create,
   show, and end-session calls.
2. Session credentials stay in ability args only where required by the
   session-control contract; they are never used as routing identity.
3. The timeout proof must observe a terminal public session view, not only a
   local timer or source-code assertion.
4. `terminal_receipt.reason_code` must be `session_expired` and
   `terminal=true`.
5. A post-timeout `end_session` call must be idempotent and preserve the
   original timeout terminal receipt.
6. The E2E harness must remain bounded and explicit: no implicit daemon
   startup, no broad filesystem mutation, and no product-complete claim.
