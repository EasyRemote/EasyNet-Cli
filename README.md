# EasyNet CLI

Hub device management, EAL ability orchestration, and MCP server for [EasyNet Axon](https://github.com/EasyRemote/EasyNet-Axon).

## Install

```bash
cargo install --path .
```

## Quick Start

```bash
# Start local runtime and join a Hub
easynet start --hub axon://hub.easynet.run:50084 --tenant myteam --foreground

# List devices
easynet devices

# List abilities
easynet abilities

# Execute a command on a remote device
easynet exec gpu-rig -- nvidia-smi

# Deploy an ability
easynet deploy ./skills/photo-capture --to edge-cam-01

# Invoke an ability
easynet invoke edge-cam-01 photo.capture --args '{"resolution": "4k"}'

# Run an EAL mission
easynet mission run examples/daily-report.eal

# Inspect compiled Mission IR without executing
easynet mission run examples/diamond.eal --emit-ir

# Start MCP server for Claude Code / Codex
easynet mcp-server --tenant myteam
```

## EAL (EasyNet Ability Language)

EAL is a DSL for distributed ability orchestration. It describes what to call, on which node, and how data flows between steps.

```eal
mission "daily-report" {
  let photo = call "photo.capture" on "edge-cam-01" with {
    resolution = "4k"
  } timeout 30

  let config = call "config.fetch" on "coordinator"

  let result = call "model.inference" on "gpu-rig" with {
    input = photo.output,
    model_config = config.output
  } timeout 120 retries 2 on_failure retry

  let report = call "data.collect" on "home-server" with {
    data = result.output,
    template = "daily-report"
  }
}
```

Key properties:
- **Dependencies are inferred** from variable references (`photo.output`), not manually declared
- **Phase partitioning** via topological layering — independent steps run in parallel
- **Compiles to Mission IR v2** — serializable, backend-agnostic intermediate representation
- **Client-side interpreter** (MVP) drives Axon `Invoke` RPCs; future target is server-side MissionControl v2

## Language Layering

```
AAL (Agent Assembly Language)     ← future: agent behavior
EAL (EasyNet Ability Language)    ← this project: ability orchestration
Mission IR v2                     ← serializable execution plan
Axon Invoke / InvokeFanOut        ← existing execution primitives
Axon Runtime + Federation + Hub   ← existing infrastructure
```

## MCP Server

Configure Claude Code to use the EasyNet MCP server:

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

Available tools: `hub_status`, `list_devices`, `get_device_detail`, `list_all_abilities`, `search_abilities`, `deploy_ability`, `execute_command`, `invoke_ability`, `run_mission`, `manage_device`, `uninstall_ability`.

## Examples

See [`examples/`](./examples/) for EAL programs:
- `hello.eal` — single step
- `parallel.eal` — independent steps, one phase
- `pipeline.eal` — linear chain A → B → C
- `diamond.eal` — diamond dependency A → B, A → C, B+C → D
- `daily-report.eal` — full multi-node orchestration showcase

## License

Apache-2.0
