<p align="center">
  <a href="https://github.com/EasyRemote"><img src="https://avatars.githubusercontent.com/u/213722898?s=200&v=4" width="200" height="200" alt="EasyRemote"></a>
</p>

<h1 align="center">easynet</h1>

<p align="center">
  Open-world capability runtime for AI-era execution: publish, invoke, audit, and orchestrate abilities across owners, devices, models, tools, and services.
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

## Why EasyNet exists

The next software runtime is no longer made only of deterministic APIs,
database operators, and scheduled jobs. Production workflows now call LLMs,
agents, MCP tools, external SaaS services, private models, local devices, and
human-in-the-loop systems. These capabilities are powerful, but they are
scattered across owners, machines, networks, and trust boundaries.

Today, a capability usually lives behind one framework, one API key, one
server, or one agent session. It has no stable network identity, no portable
execution contract, no verifiable receipt, and no durable execution history
that can be optimized over time.

EasyNet gives every capability a network address and an execution contract.

A capability can stay where it belongs: on the owner's machine, inside the
owner's model runtime, behind the owner's SaaS account, or attached to the
owner's device. Other software can discover and invoke it through a signed,
auditable protocol call. The caller receives a result and a receipt, not the
underlying model, code, credential, or data source.

> A research workflow needs protein-folding inference but has no local GPU. A
> remote owner publishes a fold capability from their own GPU box. The workflow
> discovers the capability, invokes it through EasyNet, receives the result, and
> keeps a receipt for audit and settlement. The model weights never leave the
> owner's machine.

**In open-world execution, the missing layer is capability management:**

| Problem | EasyNet's answer |
|---|---|
| Capabilities are trapped inside one runtime, API, account, or framework | **Ability** gives each capability a stable network contract |
| Callers cannot safely invoke capabilities owned by others | **Invocation** carries caller, callee, ability, subject, nonce, causal context, and args as a signed object |
| Execution is hard to audit, replay, or settle | **Receipt** records the verifiable terminal fact of the call |
| Similar workflows repeatedly re-plan from scratch | Execution traces become reusable optimization material |
| Private models and devices sit behind NAT or local trust boundaries | The capability stays with its owner; EasyNet routes the call |

**One line:** EasyNet is the open-world capability network: a protocol and
runtime stack for publishing, invoking, auditing, and eventually optimizing
AI-era capabilities across ownership and network boundaries.

## What is this?

`easynet` is the device runtime and operator CLI for EasyNet. It starts
`easynet-daemon`, joins an Axon Hub, publishes local capabilities as governed
Abilities, dispatches complete signed Invocations, and records execution facts
as receipts.

**Four things in one binary:**

1. **Daemon runtime**: `easynet runtime start` launches `easynet-daemon`, the
   product runtime that owns device lifecycle, local policy, plugins, ability
   registration, dispatch, and receipt projection.

2. **CLI**: `easynet devices`, `easynet exec gpu-rig -- nvidia-smi`,
   `easynet deploy ./skill --to edge-cam`, and `easynet invoke ...` provide
   direct control over devices and abilities.

3. **EAL compiler**: EAL (EasyNet Ability Language) is a DSL for distributed
   ability orchestration. Write a `.eal` file, and the compiler infers a
   dependency DAG from variable references, partitions it into parallel phases,
   and compiles it to **Mission IR v2**, a serializable execution plan.

4. **MCP server**: `easynet mcp-server` exposes Hub-level tools over stdio, so
   local AI development environments can discover devices, deploy abilities,
   run commands, and orchestrate missions without leaving the IDE.

**Why this matters:** EasyNet does not treat a capability as a local tool
hidden inside one agent session. It treats the capability as a network object:
addressable by URA, invoked through Axon, governed by daemon policy, and
closed by a receipt.

## System layers

| Layer | Repository | Responsibility |
|---|---|---|
| Protocol | [EasyNet-Axon](https://github.com/EasyRemote/EasyNet-Axon) | URA, Ability, Invocation, Receipt, stream/bidi, runtime semantics, SDK conformance |
| Runtime | [EasyNet-Cli](https://github.com/EasyRemote/EasyNet-Cli) | `easynet-daemon`, device lifecycle, plugins, local execution, EAL/Mission, CLI, MCP |
| Product | [EasyNet](https://github.com/EasyRemote/EasyNet) | Web platform, federation backend, operator console, dashboards, product workflows |
| Optimization | IntentDB (research direction) | Learn reusable hybrid execution plans from receipt-backed traces |

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

## Quick Start

### Join a Hub

```bash
easynet runtime start --hub axon://hub.easynet.run:50084 --tenant myteam --foreground
# ✓ easynet-daemon started
# ✓ Joined hub.easynet.run as alice-macbook
```

### Explore the fleet

```bash
easynet devices
# ┌───┬──────────────┬─────────┬──────────────┬──────────┐
# │   │ NODE         │ STATE   │ OS           │ TRUST    │
# ├───┼──────────────┼─────────┼──────────────┼──────────┤
# │ ● │ home-server  │ HEALTHY │ linux/amd64  │ TRUSTED  │
# │ ● │ gpu-rig      │ HEALTHY │ linux/arm64  │ TRUSTED  │
# │ ○ │ rpi-lab      │ OFFLINE │ linux/arm    │ PROBATION│
# └───┴──────────────┴─────────┴──────────────┴──────────┘

easynet abilities
# ┌─────────────────┬──────────────┬─────────┬────────┐
# │ ABILITY         │ NODE         │ VERSION │ STATUS │
# ├─────────────────┼──────────────┼─────────┼────────┤
# │ photo.capture   │ edge-cam-01  │ 1.2.0   │ ACTIVE │
# │ model.inference │ gpu-rig      │ 3.1.0   │ ACTIVE │
# └─────────────────┴──────────────┴─────────┴────────┘
```

### Remote execution

```bash
easynet exec gpu-rig -- nvidia-smi --query-gpu=name,memory.used --format=csv
# ┌ tunnel via hub.easynet.run (E2E encrypted)
# name, memory.used [MiB]
# NVIDIA A100-SXM4-80GB, 12453 MiB
# ✓ done
```

### Deploy an ability

```bash
easynet deploy ./skills/photo-capture --to edge-cam-01
#   publishing photo.capture@1.2.0 ... ✓
#   installed (install_id: inst-7x8k)
# ✓ activated — photo.capture is live
```

### Run an EAL mission

```bash
easynet mission run examples/daily-report.eal
# mission: daily-report (6 steps, 4 nodes, 3 phases)
#
# phase 0 (parallel):
#   [1/6] photo.capture      edge-cam-01    ✓ 1.2s  → $photo
#   [2/6] config.fetch       coordinator    ✓ 0.3s  → $config
#   [3/6] metrics.ping       monitor        ✓ 0.1s
#
# phase 1:
#   [4/6] model.inference    gpu-rig        ✓ 3.8s  → $result  (← $photo, $config)
#
# phase 2:
#   [5/6] data.collect       home-server    ✓ 0.9s  → $report  (← $result)
#
# ✓ Mission completed — 6.3s across 4 nodes (3 phases)
```

## EAL (EasyNet Ability Language)

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

### Compiler pipeline

```
.eal source → Lexer → Parser → Analyzer → Planner → Mission IR v2 → Interpreter
```

| Stage | Responsibility |
|-------|---------------|
| **Lexer** | Tokenizes keywords, literals, identifiers, comments |
| **Parser** | Recursive descent → `EalProgram` AST |
| **Analyzer** | Symbol table, VarRef validation, cycle detection (DFS), retry policy enforcement |
| **Planner** | Topological layering → phase partitioning. Dependencies **inferred** from variable references |
| **IR** | Mission IR v2 — serializable JSON with `input_refs` + `output_binding` per step |
| **Interpreter** | Parallel dispatch (`std::thread::scope`), retry with exponential backoff, structured `ExecutionTrace` |

### Key design decisions

- **Dependencies are inferred, not declared.** Write `input = photo.output` and the compiler builds the DAG. No `depends_on` arrays.
- **Phase partitioning is optimal.** Steps land in the earliest possible phase where all dependencies are resolved. Independent steps always run in parallel.
- **Mission IR v2 is the contract.** The IR is serializable, inspectable (`--emit-ir`), and backend-agnostic. Today it runs on a client-side interpreter; tomorrow on server-side MissionControl v2.
- **Retry is deterministic.** Exponential backoff with SHA-256-based jitter seeded by `(step_id, attempt)` — reproducible across runs.
- **Every execution produces an audit trail.** `ExecutionTrace` captures per-step timestamps, result SHA-256 hashes, retry history, and phase structure. Output via `--trace`.

### Inspection

```bash
# Compile to IR without executing
easynet mission run examples/diamond.eal --emit-ir

# Execute with full audit trace
easynet mission run examples/daily-report.eal --trace
```

## Language Layering

```
┌──────────────────────────────────────────────┐
│  AAL (Agent Assembly Language)               │  future — agent behavior
│  goals, planning, memory, decisions          │
├──────────────────────────────────────────────┤
│  EAL (EasyNet Ability Language)              │  this project
│  call, let, data flow, failure policy        │
│  compiles to Mission IR v2                   │
├──────────────────────────────────────────────┤
│  Mission IR v2                               │  serializable execution plan
│  steps + input_refs + output_binding         │
├──────────┬───────────────────────────────────┤
│ Client   │  MissionControl v2 (future)       │
│ Interp.  │  server-side, stateful,           │
│ (current)│  checkpoint, resume               │
├──────────┴───────────────────────────────────┤
│  Axon Runtime + Federation + Hub             │  existing infrastructure
└──────────────────────────────────────────────┘
```

EAL describes **distributed ability execution**. AAL (future) will describe **agent behavior** — goals, planning, memory — and emit EAL programs as its execution substrate.

## MCP Server

`easynet mcp-server` runs a Hub-level MCP server on stdio. Configure Claude Code:

```json
{
  "mcpServers": {
    "easynet": {
      "command": "easynet",
      "args": ["mcp-server", "--tenant", "myteam"]
    }
  }
}
```

### Install for Claude / Codex

Instead of editing config by hand, use:

```bash
# Claude Code: updates ~/.claude/settings.json
easynet mcp-install claude --name easynet --tenant myteam

# Bind a server to a single device (node_id); run twice for two devices/agents
easynet mcp-install claude --name easynet-edge-a --tenant myteam --bound-node edge-a --agent agent-a
easynet mcp-install claude --name easynet-edge-b --tenant myteam --bound-node edge-b --agent agent-b
```

### Available tools

| Tool | Description |
|------|------------|
| `hub_status` | Hub connection, node/ability counts |
| `list_devices` | All devices across federation |
| `get_device_detail` | Device info + installed abilities |
| `list_all_abilities` | Abilities across all nodes |
| `search_abilities` | Find by name pattern |
| `list_a2a_agents` | List A2A agents in tenant |
| `get_a2a_agent_card` | Fetch A2A agent card |
| `send_a2a_task` | Send an A2A skill task to an agent |
| `deploy_ability` | Publish → install → activate pipeline |
| `execute_command` | One-shot command on remote device |
| `invoke_ability` | Invoke ability on any federated node |
| `run_mission` | Compile + execute EAL program |
| `manage_device` | Drain / disconnect device |
| `uninstall_ability` | Remove ability from device |

## CLI Reference

```
easynet runtime start   --hub <endpoint> [--tenant T] [--label L] [--token T] [--foreground]
easynet runtime stop
easynet status
easynet devices  [--state online|offline] [--format table|json]
easynet abilities [--node N] [--format table|json]
easynet exec     <node> -- <command...>
easynet deploy   <path> --to <node>
easynet invoke   <node> <ability> [--args JSON]
easynet mission  run <file.eal> [--emit-ir] [--trace]
easynet mcp-server [--endpoint URL] [--tenant T] [--bound-node N] [--agent A]
easynet mcp-install <claude|codex> [--name NAME] [--tenant T] [--bound-node N] [--agent A]
```

## Examples

See [`examples/`](./examples/) for EAL programs:

| File | Pattern | Phases |
|------|---------|--------|
| `hello.eal` | Single step | 1 |
| `parallel.eal` | Independent steps | 1 (all parallel) |
| `pipeline.eal` | Linear chain A → B → C | 3 (fully sequential) |
| `diamond.eal` | Diamond: A → B, A → C, B+C → D | 3 (B∥C in phase 1) |
| `daily-report.eal` | Full multi-node orchestration | 3 (4 parallel in phase 0) |

## License

Apache-2.0 — see [LICENSE](https://github.com/EasyRemote/EasyNet-Cli/blob/main/LICENSE).

## Author

[Silan Hu](https://github.com/Qingbolan) · [silan.hu@u.nus.edu](mailto:silan.hu@u.nus.edu)
