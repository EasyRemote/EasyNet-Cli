# API Contract

Each platform entry in `remoteapp-cross-platform-capture-e2e.sh` reports must include `scenarios[]`.

For each required target:

- `target_kind`
- `status=passed`
- `selected_resource_ura`
- `session_id`
- `capture_backend`
- `capture_scope`
- `target_binding_exact=true`
- `source_only_proof=false`
- `frame_source_id`
- `geometry_revision`
- `frames_rendered > 0`
- `selected_sentinel_rendered=true`
- `rendered_frame_probe_bound=true`
- `selected_sentinel_hash_present=true`
- `terminal_receipt_visible=true`
- `terminal_receipt_session_bound=true`

For `window` and `application`:

- `first_display_capture_started=false`
- `display_fallback_used=false`
- `unrelated_sentinel_rendered=false`
