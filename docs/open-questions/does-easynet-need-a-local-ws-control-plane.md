# Open Question — Does EasyNet-Cli need a local WebSocket control plane?

**Status:** Closed (trigger #3 fired; landed as local IPC, not WS) · **Owner:** Silan Hu · **Date:** 2026-04-23 · **Closed:** 2026-04-25

## Why this is an open question, not a plan item

Earlier drafts of the plan listed "PR-6 — daemon + WS control plane + pidfile + bearer auth" as a standalone item. Two facts, established by grepping the code and tracing actual customer paths, contradicted that framing:

1. **`run_daemon()` already exists** in `src/facade/cli/heartbeat.rs:428`. The CLI is not daemon-less. What it is not is a *session*-owning daemon — heartbeat only keeps the node's registration alive.
2. **No local client connects to a WS on the CLI.** The Frontend reaches the backend over HTTP; the backend reaches the CLI as an Axon node; cross-agent calls ride the Axon invocation layer, not a local WS. Mobile is explicitly out of scope (retracted in earlier plan audits). `easynet attach` / `easynet watch` / `easynet tail` verbs do not exist, neither as implementations nor as named customer asks.

Given that, "WS control plane" has no user. Merging it into PR-7 would drag a ~500-line subsystem in with no consumer — the ACP pattern we agreed is forbidden now.

## What the former PR-6 actually demanded

The EAL control-flow RFC §7 (`docs/rfc/eal-control-flow-v1.md`) names five behaviours that differ between daemon-online and daemon-offline:

1. `services/loop_exec` scheduling
2. `services/chat` broadcast
3. Mid-iteration timeline streaming
4. `permit` interactive flow
5. Hot-interrupt from client

Tier A (infrastructure — absorbed into PR-7):

- Tokio runtime owner (required for async fs + broadcast)
- Long-lived process as session owner (session outlives one CLI invocation)
- In-process pub/sub for services/chat broadcast

Tier A cannot be deferred — PR-7 cannot ship Session + Timeline without it.

Tier B (user-facing surface — deferred here):

- WS server accepting local TCP connections
- Bearer-token auth against those connections
- `permit` interactive flow (RFC §7 locates this in PR-10, not PR-7, anyway)
- Hot-interrupt protocol for clients

Tier B cannot ship without a named client. None exists.

## What would move this to a plan item

Any one of:

1. **A concrete `easynet attach` / `easynet watch` / `easynet tail` verb request** with a user story naming why a foreground invocation needs to connect to a background session (tmux-style). Who is that operator, what are they trying to see, why does `runs/<id>/timeline.jsonl` tail not suffice?
2. **A mobile-side client** reverses the earlier scope decision to exclude mobile. Would require re-opening that scope call.
3. **A local UI** (e.g. a future desktop EasyNet client that shells out to `easynet` commands and wants to subscribe to their progress without polling) names a specific protocol need.
4. **PR-10 `permit` interactive flow** writes a spec that requires a local WS listener. RFC §7 currently frames `permit` as an EAL construct inside missions, not as an inter-process RPC — but the spec is a forward-looking doc and may grow a local-client clause.

Without one of the four, Tier B stays out of the plan.

## What was considered and rejected

- **"Build it now, clients will come later."** This is the ACP-driver pattern. AgentAdapter trait is already in place so a late-arriving client could shim in; the WS surface does not need pre-building for optionality to be preserved.
- **"The heartbeat daemon is close enough, just extend it."** No — the heartbeat process runs a sync polling loop over `ReconnectingBridge`; it has no tokio runtime, no session state, no broadcast. Extending it to session ownership is a rewrite, not an extension, and that rewrite is exactly what PR-7 Tier A is.

## If it becomes a plan item

The shape is roughly what the former PR-6 sketched, minus the Paseo-adjacent parts:

- `transport/ws_server.rs` — axum + tokio-tungstenite, bearer token from pidfile, accept loop
- `transport/mux.rs` — channel 0 JSON control, channels 1+ reserved
- Hello/welcome version + capability negotiation
- Permission round-trip if a `permit` client exists by then

But each of those lines is designing without a consumer today. Revisit when a consumer names itself.

## Blocker status for other PRs

- **PR-7** (session + timeline): not blocked. Tier A absorbed; Tier B explicitly excluded.
- **PR-10** (EAL + services): not blocked. RFC §7 daemon-online/offline behaviour diffs can all be implemented without a WS surface — timeline.jsonl tail handles the streaming need; broadcast is in-process to other services.

## Log

| Date       | Event                                                                                 |
|------------|---------------------------------------------------------------------------------------|
| 2026-04-23 | Former PR-6 task deleted (#8). Tier A absorbed into PR-7 (#7). Tier B opened here.   |
| —          | Revisit: **trigger-based**, when any of the four customer signals above surfaces.    |
| 2026-04-25 | **Closed.** Trigger #3 fired: EasyNet-Client desktop UI is the named local consumer. Landed as a Unix-Domain-Socket / Named-Pipe IPC control plane (length-prefixed JSON, filesystem-permission auth, no bearer token), **not** as a WebSocket server. Rationale captured in `docs/design/daemon-layers-v1.md` §"Local IPC interface — UDS/Named Pipe": client & daemon are same-host by construction, so TCP+WS has no advantage over UDS; filesystem permissions give stronger cross-user isolation than a bearer token; no port footprint. Triggers #1/#2/#4 status unchanged (no `easynet attach` verb on a foreground command, no mobile client in scope, no `permit` WS clause — `permit` still lives inside missions). |
