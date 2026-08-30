# API Contract

`frontend-remoteapp-product-flow-e2e.sh` reports must include `frontend_flow_summary`.

Required fields:

- `target_kind=both`
- `hub_api_ready=true`
- `product_runtime_ready=true`
- `frontend_typechecked=true`
- `ui_flow_exercised=true`
- `browser_lifecycle_verified=true`
- `cross_device_distinct_devices=true`
- `permission_subject_checked=true`
- `target_picker_fresh=true`
- `window_frame_rendered=true`
- `application_frame_rendered=true`
- `window_view_only_input_checked=true`
- `application_view_only_input_checked=true`
- `end_session_lifecycle_verified=true`

The summary must also expose `passed_steps` so product-completion can cross-check it against the required step list.
