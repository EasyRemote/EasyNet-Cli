# System Abilities v1

> Device-level abilities published by `easynet-daemon` under the
> `system.<feature>[.<verb>]` namespace. Distinct from agent
> abilities (`<agent>.chat`, `<agent>.<verb>`) — system abilities
> belong to the *node*, not to any one registered AI agent.

## 1. Naming + discovery

- Wire name: `system.<feature>.<verb>` (`system.session.attach`,
  `system.permission.decide`, …).
- Discovery: each daemon publishes the list under the
  `a2a.system_skills_json` node label (alongside the existing
  `a2a.agents_json`). Parsers that only know the v2 envelope
  ignore the new key without disruption.
- Schema source of truth: `schemas/system/*.proto` (PR-DAEMON
  Commit 2). v1 wire format is JSON (proto JSON mapping); v2
  may flip to proto bytes.

## 2. v1 ability surface

| ability                          | mode    | summary                                                            | landed by  |
|----------------------------------|---------|--------------------------------------------------------------------|------------|
| `system.ping`                    | RPC     | echo args + daemon timestamp; round-trip smoke                     | PR-SYS     |
| `system.session.list`            | RPC     | snapshot every Session known to this daemon                        | PR-ATTACH  |
| `system.session.attach`          | Stream  | replay TimelineEvent frames from a session, then tail live         | PR-ATTACH  |
| `system.permission.subscribe`    | Stream  | snapshot pending PermissionRequests; live tail (PR-INVOCATION)     | PR-PERM    |
| `system.permission.decide`       | RPC     | deliver decision (allow / deny / allow_once)                       | PR-PERM    |
| `system.discuss.create`          | RPC     | spin up a multi-agent discussion room                              | PR-DISCUSS |
| `system.discuss.post`            | RPC     | append one turn to a room                                          | PR-DISCUSS |
| `system.discuss.subscribe`       | Stream  | replay turns ≥ since_seq                                           | PR-DISCUSS |
| `system.schedule.add`            | RPC     | register a cron schedule with misfire policy                       | PR-SCHED   |
| `system.schedule.list`           | RPC     | list every schedule                                                | PR-SCHED   |
| `system.schedule.remove`         | RPC     | delete by id                                                       | PR-SCHED   |
| `system.schedule.enable`         | RPC     | toggle enabled flag                                                | PR-SCHED   |
| `system.loop.create`             | RPC     | register a worker+verify loop bounded by max_iters                 | PR-LOOP    |
| `system.loop.status`             | RPC     | fetch loop instance state                                          | PR-LOOP    |
| `system.loop.subscribe`          | Stream  | replay buffered loop frames, then tail live controller output      | PR-LOOP    |
| `system.loop.cancel`             | RPC     | cancel an in-flight loop                                           | PR-LOOP    |

## 3. Schema layout

Every ability has an `input_schema` (JSON Schema, top-level
object) and an optional `output_schema`. v1 emits explicit `null`
for missing optional fields so the discovery JSON is byte-stable
across rebuilds.

Per-ability TOML manifests live under `abilities/system/`:

```
abilities/system/
├── ping.ability.toml
├── session.list.ability.toml
├── session.attach.ability.toml
├── permission.subscribe.ability.toml
├── permission.decide.ability.toml
├── discuss.create.ability.toml
├── discuss.post.ability.toml
├── discuss.subscribe.ability.toml
├── schedule.add.ability.toml
├── schedule.list.ability.toml
├── schedule.remove.ability.toml
├── schedule.enable.ability.toml
├── loop.create.ability.toml
├── loop.status.ability.toml
├── loop.subscribe.ability.toml
└── loop.cancel.ability.toml
```

## 4. Forward compatibility

- Adding a new ability is purely additive — push a new
  `system::<feature>::register(reg, sub_service)` line into
  `runtime::system::build_registry_with_services` and the
  discovery / dispatch surfaces both pick it up.
- Renaming a wire name is a breaking change. Bump the
  per-ability schema_version + run a coordinated Client release.
- Adding an optional field to a request or response is additive;
  Clients that don't know the field must ignore it (JSON
  forward-compat rule the v2 envelope already follows).

## 5. v2 deltas (out of scope here)

- Live tail for `system.session.attach`, `system.permission.subscribe`,
  `system.loop.subscribe`, `system.discuss.subscribe`.
- Schedule tick runner that fires schedules at next-fire instants
  (PR-INVOCATION-EXEC-UNITY's deferred half).
- Proto-encoded args/responses replacing the JSON v1 wire format.
