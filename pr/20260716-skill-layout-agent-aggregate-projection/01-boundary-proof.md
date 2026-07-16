# Boundary Proof

## Owner

`src/daemon/persistence/agent_aggregate.rs` owns read projections derived from
registered Agent persistence.

## Boundary

Skill resource code owns skill package filesystem layout and mutation behavior.
It may consume an aggregate projection that says which skill layout class a
registered Agent uses, but it must not consume `AgentRegistry` row types as its
public input model.

## Invariants

1. `AgentRegisteredWorkspace` exposes `AgentSkillLayout`, not raw
   `agent_registry::AgentType`.
2. `skill.publish`, `skill.list`, and shared skill store helpers do not mention
   `agent_registry::AgentType` in production code.
3. `skill.list` preserves scoped behavior by resolving only included owner
   workspaces through the aggregate snapshot.
4. Claude Code still maps to `.claude/skills`; Codex and external layouts still
   map to `skills`.
5. Global skill pool labels and public `skill.list` response shape are
   unchanged.
