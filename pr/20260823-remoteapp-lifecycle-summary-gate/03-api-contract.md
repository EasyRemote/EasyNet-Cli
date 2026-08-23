# API Contract

Required report summary fields:

- common:
  - `target_kind`
  - `selected_resource_ura`
  - `session_id`
  - lifecycle-specific `lifecycle_summary`
- timeout:
  - `terminal_state=closed`
  - `terminal_reason=session_expired`
  - `terminal_receipt_visible=true`
  - `terminal_receipt_session_bound=true`
  - `idempotent_end=true`
- cancel:
  - `terminal_state=closed`
  - `terminal_reason=user_cancelled`
  - `terminal_receipt_visible=true`
  - `terminal_receipt_session_bound=true`
  - `idempotent_cancel=true`
- permission revoke:
  - `proof_mode=real_platform_permission_revoke`
  - `operator_revoke_required=true`
  - `terminal_state=closed`
  - `terminal_reason=target_permission_revoked`
  - `consent_phase=revoked`
  - `event_order=target_permission_revoked_before_media_lost_before_closed`
- resume:
  - `proof_mode=lease_refresh_resume`
  - `lease_extended=true`
  - `waited_past_original_lease=true`
  - `survived_original_lease=true`
  - `non_terminal_after_refresh=true`
  - `cleanup_terminal_reason=resume_e2e_cleanup`
