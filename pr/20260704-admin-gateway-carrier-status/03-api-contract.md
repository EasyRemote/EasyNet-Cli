# API Contract

## C ABI Functions

- `easynet_admin_build_agent_list_invocation(handle, request_json, out_json)`
- `easynet_admin_build_agent_start_invocation(handle, request_json, out_json)`
- `easynet_admin_build_agent_stop_invocation(handle, request_json, out_json)`
- `easynet_admin_build_agent_refresh_invocation(handle, request_json, out_json)`
- `easynet_admin_build_session_list_invocation(handle, request_json, out_json)`
- `easynet_admin_project_gateway_status(handle, status_json, out_json)`
- `easynet_admin_project_agent_records(handle, agents_json, out_json)`
- `easynet_admin_project_agent_lifecycle_result(handle, result_json, out_json)`

## Carrier Input

All carrier requests require explicit Invocation fields:

- `caller_ura`
- `callee_ura`
- `subject_ura`
- `descriptor_version`
- `nonce_base64`
- `causal_context`

Profile-specific arguments:

- `agent.start`: `name`, `agent_type` or `entry`, optional `model`, `label`,
  `command`, `command_args`, materialization flags.
- `agent.stop`: `name` or `agent_ura`.
- `agent.refresh`: optional `name`.
- `agent.list`: no profile-specific fields.
- `session.list`: optional `include_terminated`.

## Projection Output

`GatewayStatus` contains:

- `profile`
- `gateway_id`
- `ready`
- `state`
- `process_live`
- `control_ready`
- `runtime_ready`
- `directory_ready`
- `trust_ready`
- `public_listener_ready`
- `listeners`
- `identity`
- `metadata`

`AgentRecord` contains:

- `agent_ura`
- `owner_ura`
- `device_ura`
- `name`
- `state`
- `runtime`
- `model`
- `label`
- `abilities`
- `metadata`

## Error Rules

Malformed JSON, missing required tuple fields, invalid URAs, invalid
descriptor versions, invalid nonce, non-object causal context, invalid
`agent.stop` target combinations, and malformed gateway facts return
`ERR_INVALID_ARG`. Invalid handles return `ERR_INVALID_HANDLE` after zeroing the
output pointer.
