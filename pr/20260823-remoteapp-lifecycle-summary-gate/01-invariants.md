# Invariants

- Each RemoteApp session lifecycle report must bind `selected_resource_ura`, `session_id`, and target kind.
- Terminal lifecycle cases must expose terminal state, reason, terminal receipt visibility, and receipt/session binding.
- Idempotent lifecycle cases must expose idempotency preservation explicitly.
- Resume must remain non-terminal through lease refresh and public show_session after the original lease, then close with cleanup receipt.
- Permission revoke must prove real platform/operator revoke mode and event ordering, not a simulated debug path.
- Aggregate product completion remains the only product-complete claim.
