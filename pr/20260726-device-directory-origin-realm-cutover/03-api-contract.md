# API contract

## Input

`DirectoryEntry` JSON from `federation.discover` may carry
`origin_realm: Option<String>`.

## Output

`easynet device list --format json` rows carry:

- `node_id`
- `agent_ura`
- `display_name`
- `state`
- `online`
- `is_self`
- `paired`
- optional `origin_realm`
- optional `hub_endpoint`

They do not carry `tenant_id`.

## Error rule

This cutover does not weaken existing fail-closed validation for `agent_ura`,
`node_id`, or `status`.
