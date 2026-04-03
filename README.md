<p align="center">
  <a href="https://github.com/EasyRemote"><img src="https://avatars.githubusercontent.com/u/213722898?s=200&v=4" width="200" height="200" alt="EasyRemote"></a>
</p>

<h1 align="center">easynet</h1>

<p align="center">
  CLI + MCP server for <strong>EasyNet Axon</strong> — manage distributed devices, orchestrate abilities across edge nodes, and give Claude Code direct access to your fleet.
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

## What is this?

`easynet` is a single binary that turns a fleet of edge devices into a programmable network. It speaks to an **Axon Hub** — the coordination layer that routes ability invocations across devices behind NAT, without any of them needing a public IP.

**Three things in one binary:**

1. **CLI** — `easynet devices`, `easynet exec gpu-rig -- nvidia-smi`, `easynet deploy ./skill --to edge-cam`. Ten subcommands for real-time device and ability management.

2. **EAL compiler** — EAL (EasyNet Ability Language) is a DSL for distributed ability orchestration. Write a `.eal` file, the compiler infers a dependency DAG from variable references, partitions it into parallel phases, and compiles it to **Mission IR v2** — a serializable, backend-agnostic execution plan.

3. **MCP server** — `easynet mcp-server` exposes 11 Hub-level tools over stdio, so Claude Code or Codex can discover devices, deploy abilities, run commands, and orchestrate missions without leaving the IDE.

**Why this matters:** Most distributed execution tools force you to choose between a CLI (manual, one-shot) and a programmable API (code-heavy, no visibility). EAL sits in between — it's a language with compiler-level guarantees (type-safe variable references, cycle detection, phase-optimal parallelism) that produces an inspectable IR and a structured audit trail.

## Install

```bash
cargo install --path .
```

> **Note:** Requires a local or remote Axon runtime. The `easynet start` command auto-spawns one and joins the Hub.

## Quick Start

### Join a Hub

```bash
easynet start --hub axon://hub.easynet.run:50084 --tenant myteam --foreground
# ✓ Axon runtime started on http://127.0.0.1:50123
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

### Available tools

| Tool | Description |
|------|------------|
| `hub_status` | Hub connection, node/ability counts |
| `list_devices` | All devices across federation |
| `get_device_detail` | Device info + installed abilities |
| `list_all_abilities` | Abilities across all nodes |
| `search_abilities` | Find by name pattern |
| `deploy_ability` | Publish → install → activate pipeline |
| `execute_command` | One-shot command on remote device |
| `invoke_ability` | Invoke ability on any federated node |
| `run_mission` | Compile + execute EAL program |
| `manage_device` | Drain / disconnect device |
| `uninstall_ability` | Remove ability from device |

## CLI Reference

```
easynet start   --hub <endpoint> [--tenant T] [--label L] [--token T] [--foreground]
easynet stop
easynet status
easynet devices  [--state online|offline] [--format table|json]
easynet abilities [--node N] [--format table|json]
easynet exec     <node> -- <command...>
easynet deploy   <path> --to <node>
easynet invoke   <node> <ability> [--args JSON]
easynet mission  run <file.eal> [--emit-ir] [--trace]
easynet mcp-server [--endpoint URL] [--tenant T]
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
