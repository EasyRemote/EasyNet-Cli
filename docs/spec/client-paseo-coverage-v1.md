# Client Paseo Coverage v1

> Map of which Paseo features the v10.5 R1 plan delivers vs. which
> are explicitly out of scope. Used by the EasyNet-Client repo to
> scope its UI work.

## 1. In scope (covered by v1 PRs)

| Paseo feature      | EasyNet ability(ies)                                                | PR              |
|--------------------|---------------------------------------------------------------------|-----------------|
| Attach session     | `system.session.list`, `system.session.attach`                      | PR-ATTACH       |
| Permission approval| `system.permission.subscribe`, `system.permission.decide`            | PR-PERM         |
| Multi-agent room   | `system.discuss.create`, `system.discuss.post`, `system.discuss.subscribe` | PR-DISCUSS |
| Cron schedule      | `system.schedule.{add,list,remove,enable}`                          | PR-SCHED        |
| Worker+verify loop | `system.loop.{create,status,subscribe,cancel}`                      | PR-LOOP         |

Combined with the existing `<agent>.chat` per-agent ability, this
is roughly 55% of Paseo's user-facing feature surface.

## 2. Explicitly out of scope (to be filed as separate plans)

The following are NOT in v1. Each is its own plan when the time
comes; bundling them in here would be over-promise.

- **Worktree / scripts / services + HTTP port proxy** — Paseo's
  long-running per-room services. v1 daemon does not host
  proxies.
- **GitHub integration** — PR / issue attach, check / review
  reads, branch switching. Integrates with an external system;
  needs its own scope decision.
- **Multi-provider model picker** — runtime model switching
  beyond what the existing claude_code / codex / opencode driver
  trio offers.
- **Voice (STT/TTS)** — Paseo has bidirectional voice; v1 does
  not.
- **File explorer + code preview** — not a runtime concern; Client
  side only.
- **Agent-as-MCP spawn/manage** — Paseo exposes its agents as MCP
  tools spawnable by other agents. v1 mcp/ only emits agents-as-
  tools, not spawn/manage.
- **Follow-up `send`** — persistent in-room agent dialogue. The
  current `agent send` verb is single-shot.
- **Agent feature toggles** (thinking mode, reasoning effort) —
  per-runtime knobs. Out of scope for the daemon abstraction.

## 3. iOS / Android

Out of scope for v1 daemon mode (the `easynet-daemon` process
model conflicts with iOS App sandboxing; Android background
service constraints make App-as-daemon non-trivial). Two future
paths, each its own plan:

1. **Remote-observe mode** — App is a pure UI; connects to a
   peer device's daemon over LAN/VPN or Hub relay.
2. **In-process mode** — App embeds the library and registers as
   an Axon node directly. Conflicts with the "Client never
   touches Axon" definition; needs an architecture re-audit.

## 4. Cross-repo dependency surface

The EasyNet-Client repo's bindings consume:

- The C ABI defined in `docs/spec/ffi-abi-v2.md`.
- The wire framing defined in `docs/spec/control-plane-v1.md`.
- The ability set defined in `docs/spec/system-abilities-v1.md`.

Schema source: `schemas/*.proto`. Generate per-language types
with `protoc` in the Client repo's CI.

## 5. Daemon-only paths (not exposed to Client)

Some daemon responsibilities are intentionally invisible to
Client:

- Pair / heartbeat with the Hub (`facade::cli::run_daemon`).
- Axon node identity + registration label maintenance.
- Filesystem persistence (`tenants/<id>/{schedules,loops,...}/`).

A Client wanting visibility into "is the device online?" should
call `system.ping` and observe the round-trip latency; the
heartbeat sub-system is not surfaced via FFI.
