# Architecture

## Owner Boundary

- `AbilityManifest` owns ability name, description, executor binding and
  `input_schema`.
- `AgentAbilitySpec` is a discovery/hint DTO derived from manifests.
- MCP/A2A/dispatch paths consume schemas from descriptors or manifests, not
  from the discovery DTO.

## Change

Drop the retained `parameters` field and getter from `AgentAbilitySpec`.
Keep the constructor's non-object schema validation so bad manifests still fail
closed before visibility.

## Obsolete Path Removed

The schema payload is no longer duplicated inside `AgentAbilitySpec`.
