<p align="center">
  <a href="https://github.com/EasyRemote"><img src="https://avatars.githubusercontent.com/u/213722898?s=200&v=4" width="200" height="200" alt="EasyRemote"></a>
</p>

<h1 align="center">EasyNet Runtime</h1>

<p align="center">
  Open capability runtime for AI agents: publish, invoke, audit, and orchestrate governed abilities across owners, devices, models, tools, and services.
</p>

<p align="center">
  EasyNet is not a VPN, an ISP, or a generic networking tool. It is the runtime layer that turns private capabilities into addressable, policy-governed, receipt-backed network objects.
</p>

<p align="center">
  <a href='https://github.com/EasyRemote/EasyNet-Axon'><img src='https://img.shields.io/badge/EasyNet-Axon-00d9ff?style=for-the-badge&labelColor=0f172a'></a>
  <a href='https://github.com/EasyRemote/EasyNet-Cli/blob/main/LICENSE'><img src='https://img.shields.io/badge/License-Apache_2.0-f97316?style=for-the-badge&labelColor=0f172a'></a>
</p>
<p align="center">
  <img src='https://img.shields.io/badge/Rust-1.75+-dea584?style=for-the-badge&logo=rust&logoColor=white&labelColor=0f172a'>
  <img src='https://img.shields.io/badge/Platform-macOS_|_Linux-22c55e?style=for-the-badge&labelColor=0f172a'>
  <img src='https://img.shields.io/badge/EAL-v0.1-a855f7?style=for-the-badge&labelColor=0f172a'>
</p>

---

## Why EasyNet Runtime exists

The next software runtime is no longer made only of deterministic APIs,
database operators, and scheduled jobs. Production workflows now call LLMs,
agents, MCP tools, external SaaS services, private models, local devices, and
human-in-the-loop systems. These capabilities are powerful, but they are
scattered across owners, machines, accounts, networks, and trust boundaries.

Today, a capability usually lives behind one framework, one API key, one
server, or one agent session. It has no stable network identity, no portable
execution contract, no verifiable receipt, and no durable execution history
that can be optimized over time.

EasyNet Runtime gives every capability a routable identity, a governed
execution contract, and a receipt-backed invocation path.

A capability can stay where it belongs: on the owner's machine, inside the
owner's model runtime, behind the owner's SaaS account, or attached to the
owner's device. Other software can discover and invoke it through a complete,
signed, auditable protocol call. The caller receives a result and a receipt,
not the underlying model, code, credential, or data source.

> A research workflow needs protein-folding inference but has no local GPU. A
> remote owner publishes a fold capability from their own GPU box. The workflow
> discovers the capability, invokes it through EasyNet, receives the result, and
> keeps a receipt for audit and settlement. The model weights never leave the
> owner's machine.

**In open-world agent execution, the missing layer is a capability runtime:**

| Problem | EasyNet's answer |
|---|---|
| Capabilities are trapped inside one runtime, API, account, or framework | **Ability** gives each capability a stable network contract |
| Callers cannot safely invoke capabilities owned by others | **Invocation** carries caller, callee, ability, subject, nonce, causal context, and args as a signed object |
| Execution is hard to audit, replay, or settle | **Receipt** records the verifiable terminal fact of the call |
| Similar workflows repeatedly re-plan from scratch | Execution traces become reusable optimization material |
| Private models and devices sit behind NAT or local trust boundaries | The capability stays with its owner; EasyNet routes the call |

**One line:** EasyNet Runtime is the open capability runtime for AI agents:
publish governed abilities, invoke them across ownership boundaries, and close
every execution with a verifiable receipt.

## Core hypothesis

If AI remains chat, EasyNet is unnecessary.

EasyNet exists for the moment AI becomes an actor across resources it does not
own: private devices, private models, local context, SaaS accounts, enterprise
networks, and human-controlled systems.

At that boundary, tool access is not enough. The system must know who called,
which capability was invoked, what subject it acted on, which authority admitted
it, what actually executed, and which receipt closes the action.

EasyNet turns that boundary into a runtime model: Ability, Invocation,
Admission, and Receipt.

## What is this?

`easynet` is the operator CLI for EasyNet Runtime. It starts
`easynet-daemon`, joins an Axon Hub, publishes local capabilities as governed
Abilities, dispatches complete signed Invocations, and records execution facts
as receipts.

**Four things in one binary:**

1. **Daemon runtime**: `easynet runtime start` launches `easynet-daemon`, the
   product runtime that owns device lifecycle, local policy, plugins, ability
   registration, dispatch, and receipt projection.

2. **CLI**: `easynet device ...`, `easynet ability ...`,
   `easynet plugin ...`, and `easynet invocation ...` expose the operator
   surface for devices, abilities, plugins, and audit records.

3. **EAL compiler**: EAL (EasyNet Ability Language) is a DSL for distributed
   ability orchestration. Write a `.eal` file, and the compiler infers a
   dependency DAG from variable references, partitions it into parallel phases,
   and compiles it to **Mission IR v2**, a serializable execution plan.

4. **MCP server**: `easynet mcp serve` exposes the local runtime over stdio so
   a co-located AI development environment can call governed EasyNet abilities
   through the daemon.

**Why this matters:** EasyNet does not treat a capability as a local tool
hidden inside one agent session. It treats the capability as a runtime object:
addressable by URA, invoked through Axon, governed by daemon policy, and closed
by a receipt.

## System layers

| Layer | Repository | Responsibility |
|---|---|---|
| Protocol | [EasyNet-Axon](https://github.com/EasyRemote/EasyNet-Axon) | URA, Ability, Invocation, Receipt, stream/bidi, runtime semantics, SDK conformance |
| Runtime | [EasyNet-Cli](https://github.com/EasyRemote/EasyNet-Cli) | EasyNet Runtime: `easynet-daemon`, device lifecycle, plugins, local execution, EAL/Mission, CLI, MCP |
| Product | [EasyNet](https://github.com/EasyRemote/EasyNet) | Web platform, federation backend, operator console, dashboards, product workflows |
| Optimization | IntentDB (research direction) | Learn reusable hybrid execution plans from receipt-backed traces |

## Network architecture roadmap

The current topology is deliberately Hub-centered: local abilities execute in
the local Runtime, remote devices maintain outbound reverse sessions to their
realm Hub, and cross-realm calls travel through governed Hub-to-Hub federation.
Realtime transports such as Remote Desktop may use Hub-mediated signaling to
establish a direct data plane with relay fallback.

The governing principle is stable across every stage: the Hub remains the
trust, routing, presence, and session-control plane, but it does not need to
carry every realtime or bulk data byte. Ability, Invocation, Admission, and
Receipt semantics remain identical regardless of the selected transport.

| Priority | Outcome | Completion criteria |
|---|---|---|
| **P0 — Hub HA and session recovery** | Preserve the current topology while removing single-process session ownership as a realm-wide failure point. | Hub failover and reconnect/resume use explicit lease epochs, fence stale owners, recover in-flight lifecycle state, and preserve one terminal Receipt. |
| **P1 — Traffic-class isolation** | Isolate control frames, unary RPC, long-lived streams, and bulk data so one traffic class cannot starve another. | Each class has bounded queues, flow control, and resource budgets; bulk or stream saturation cannot incorrectly mark a healthy device offline or block session control. |
| **P2 — Path observability** | Make route behavior and cost measurable before introducing more transport choices. | Route-labelled telemetry covers latency, Hub egress, queue saturation, reconnect rate, direct/relay selection, and terminal failure reasons with actionable dashboards and alerts. |
| **P3 — Selective P2P session data planes** | Extend direct transport only to concrete realtime or bulk sessions such as remote desktop, voice, or file transfer. | Session setup remains an authorized Invocation; direct and relay paths form one explicit recovery state machine; endpoints remain transport metadata rather than identity; every run still closes with a verifiable Receipt. |
| **P4 — Federation at scale** | Evolve beyond manually maintained peer maps without turning observed endpoints into implicit route authority. | Signed route advertisements, trust-policy acceptance, expiry/revocation, health selection, and bounded failure recovery support large federations while keeping Hub-to-Hub routing governed. |

This roadmap does not turn EasyNet into a VPN or a generic peer-to-peer mesh.
It evolves the transport beneath the governed capability runtime only where a
measured use case requires it.

## Research direction: IntentDB

EasyNet's receipt-backed execution history makes a higher-level system
possible: **IntentDB**, a database-style optimizer for open-world capability
execution.

IntentDB treats user intent as a managed object, learns recurring execution
patterns from historical traces, promotes stable patterns into deterministic
operators, and keeps uncertain reasoning only where it is still necessary. This
layer is a research direction, not a completed runtime feature in this
repository.

## Install

```bash
cargo install --path .
```

> **Note:** `easynet runtime start` launches `easynet-daemon`, which embeds the Axon Invocation runtime and joins the Hub. Product/device paths should target the daemon, not a standalone Axon reference runtime.

## Operator flow

```bash
easynet login
easynet device join <pairing-token>
easynet runtime start
easynet status
easynet device list
easynet ability list
```

Invoke a local ability by canonical Ability URA. Public ingress requires the
caller-controlled tuple fields; the CLI does not invent the subject, nonce, or
causal placement:

```bash
easynet ability invoke <ability-ura> \
  --subject <resource-ura> \
  --nonce-hex <32-hex-chars> \
  --causal-root \
  --args '{"key":"value"}'
```

Remote invocation uses the same tuple contract and additionally pins the
target runtime owner:

```bash
easynet ability invoke <ability-ura> \
  --node <device-ura> \
  --subject <resource-ura> \
  --nonce-hex <32-hex-chars> \
  --causal-root \
  --args '{"key":"value"}'
```

Run an ad-hoc command through the ability surface:

```bash
easynet ability exec <node-id> -- uname -a
```

Create and install a local plugin package:

```bash
easynet plugin init ./hello-plugin --language python
easynet plugin install ./hello-plugin
easynet plugin status <package-id>
```

Expose the local runtime to an MCP-capable client:

```bash
easynet mcp status
easynet mcp install codex --name easynet --tenant myteam
easynet mcp serve --tenant myteam
```

## EAL

EAL is a DSL for distributed ability orchestration. It separates **what to execute** from **how to schedule it** — you write the data flow, the compiler figures out the parallelism.

```eal
mission "daily-report" {
  // Phase 0: independent — dispatched in parallel
  let photo = call "photo.capture" on "edge-cam-01" with {
    resolution = "4k"
  } timeout 30

  let config = call "config.fetch" on "coordinator"

  // Phase 1: depends on photo + config — data-flow barrier
  let result = call "model.inference" on "gpu-rig" with {
    input = photo.output,
    model_config = config.output
  } timeout 120 retries 2 on_failure retry

  // Phase 2: depends on result
  let report = call "data.collect" on "home-server" with {
    data = result.output,
    template = "daily-report"
  }

  // Fire-and-forget (phase 0, no binding)
  call "metrics.ping" on "monitor" optional
}
```

Compile or run an EAL mission:

```bash
easynet mission compile examples/diamond.eal --emit-ir
easynet mission run examples/daily-report.eal --trace
```

## CLI Reference

```
easynet login
easynet device join <pairing-token>
easynet runtime start [--hub ENDPOINT] [--tenant T] [--label L] [--foreground]
easynet runtime stop | status | logs
easynet status
easynet device list | show <node-id> | remove <node-id>
easynet ability list | search <intent> | show <ability-ura>
easynet ability deploy <path> --node <node-id>
easynet ability invoke <ability-ura> --subject <resource-ura> --nonce-hex <32-hex> --causal-root [--args JSON]
easynet ability invoke <ability-ura> --node <device-ura> --subject <resource-ura> --nonce-hex <32-hex> --causal-root [--args JSON]
easynet ability stream <ability-ura> [--args JSON]
easynet ability bidi <ability-ura> [--args JSON]
easynet ability exec <node-id> -- <command...>
easynet plugin init <path> [--language python|node|go|rust|java]
easynet plugin install <path> | update <path> | remove <package-id> <version>
easynet mission compile <file.eal> [--emit-ir]
easynet mission run <file.eal> [--trace]
easynet mission list | show <run-id> | cancel <run-id>
easynet mcp status | install <claude|codex> | serve
easynet invocation list | show <request-id> | trace <request-id>
```

## License

Apache-2.0 — see [LICENSE](https://github.com/EasyRemote/EasyNet-Cli/blob/main/LICENSE).

This repository is a deliberately bounded public release of the wider EasyNet
research system. See [Public Source Release Scope](./SOURCE_RELEASE_SCOPE.md)
for what is published, what remains outside the distribution, and why releases
are staged.

## Author

[Silan Hu](https://github.com/Qingbolan) · [silan.hu@u.nus.edu](mailto:silan.hu@u.nus.edu)
