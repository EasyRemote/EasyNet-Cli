# API Contract

Each passed platform entry in `remoteapp-input-injection-e2e.sh` reports must include `input_summary`.

Required fields:

- `selected_resource_ura`
- `session_id`
- `permission_granted=true`
- `consent_scope=input_control`
- `input_scope=display_global`
- `focus_validated=true`
- `coordinate_mapping_validated=true`
- `target_geometry_revision > 0`
- `target_focus_epoch > 0`
- `source_only_proof=false`
- `policy_only=false`
- `applied_inputs[]` containing `pointer` and `keyboard`
- each applied input proves bounded latency and OS effect binding
- `stale_client_sequence_rejected=true`
- `terminal_receipt_visible=true`
- `terminal_receipt_session_bound=true`
