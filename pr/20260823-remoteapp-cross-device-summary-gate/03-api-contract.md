# API Contract

Each passed cross-device RemoteApp scenario must include `remoteapp_summary`.

Required fields:

- `caller_device_ura`
- `provider_device_ura`
- `selected_resource_ura`
- `session_id`
- `distinct_devices=true`
- `remote_target_inventory_seen=true`
- `abilities_bound=true`
- `capture_provider_bound=true`
- `capture_resource_bound=true`
- `capture_target_kind_bound=true`
- `capture_frames_captured > 0`
- `media_provider_bound=true`
- `media_resource_bound=true`
- `media_session_bound=true`
- `media_transport` is `webrtc` or `easynet_relay_webrtc`
- `media_frames_rendered > 0`
- `rendered_on_caller_device=true`
- `input_policy_checked=true`
- `input_policy_mode` is `interactive`, `view_only`, or `policy_blocked`
- `input_policy_session_bound=true`
- `terminal_receipt_visible=true`
- `terminal_receipt_session_bound=true`
