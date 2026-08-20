# Agent Ability Spec Discovery DTO

## Goal

Make `AgentAbilitySpec` represent only the discovery/hint surface that
production callers actually consume: qualified ability name and description.

## Concrete Use Case

Fresh cutover builds reported that `AgentAbilitySpec::parameters` and its
stored `parameters` field are unused in production. The schema is still
load-bearing, but its source of truth is the ability manifest and descriptor
projection path, not the prompt/discovery hint DTO.

## Non-Goals

- Do not change ability manifest schema parsing.
- Do not change daemon invocation, MCP descriptor projection, or A2A labels.
- Do not touch unrelated dirty files.
- Do not remove schema validation when constructing an `AgentAbilitySpec`.

## Acceptance Criteria

1. `AgentAbilitySpec` stores only name and description.
2. Constructor still rejects non-object schemas before accepting a manifest as
   network-visible.
3. Schema-specific tests assert against manifest data, not the discovery DTO.
4. Focused tests and architecture gates pass.
