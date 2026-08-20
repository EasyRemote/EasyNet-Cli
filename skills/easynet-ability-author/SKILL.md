---
name: easynet-ability-author
description: Author, deploy, and orchestrate EasyNet abilities and EAL missions. Use this skill when asked to create abilities for edge devices, write EAL programs, deploy to devices, or build multi-agent workflows. Covers device.ability.json schema, EAL language syntax, agent dispatch, and the full deploy/invoke lifecycle.
compatibility: Requires easynet CLI built and on PATH
metadata:
  author: easynet
  version: "1.0.0"
  axon-resource-ura: "easynet:///r/org/ability-author"
allowed-tools: Bash(*), Edit, Write, Read
---

# EasyNet Ability Author

Create, deploy, and orchestrate abilities across edge devices and AI agents.

## Concepts

**Ability**: A deployable unit of functionality. An ability has a name, a command (shell), and metadata. Once deployed to a device, it can be invoked remotely.

**EAL (EasyNet Ability Language)**: A DSL for orchestrating multiple abilities across multiple devices/agents. The compiler infers the dependency DAG, partitions into parallel phases, and produces Mission IR.

**Agent**: A registered AI CLI (Claude Code or Codex) that can be used as an EAL target alongside physical devices.

## Quick Reference

### Check environment

```bash
easynet status          # runtime status
easynet devices         # online devices
easynet abilities       # deployed abilities
easynet agent list      # registered agents
easynet agent doctor    # check agent availability
```

### Create a simple ability

Create a directory with `device.ability.json`:

```bash
mkdir -p /tmp/my-ability
cat > /tmp/my-ability/ability.json << 'EOF'
{
  "name": "my-ability-name",
  "version": "1.0.0",
  "tool_name": "my-ability-name",
  "description": "Human-readable description of what this does",
  "command": "echo '{\"result\": \"hello\"}'"
}
EOF
```

The `command` field is a shell command executed on the target device. It should output JSON.

### Deploy an ability to a device

```bash
easynet deploy /tmp/my-ability --to <node_id>
```

### Invoke a deployed ability

```bash
easynet invoke <node_id> <ability_name>
easynet invoke <node_id> <ability_name> --args '{"key": "value"}'
```

### One-shot command (no deploy)

```bash
easynet exec <node_id> -- <shell_command>
```

## EAL Language Reference

### Minimal program

```eal
mission "name" {
  let result = call "ability" on "node" with {
    key = "value"
  }
}
```

### Full syntax

```eal
mission "mission-name" {
  // Binding: capture output for data flow
  let var_name = call "function_name" on "target_node" with {
    key1 = "string value",
    key2 = 42,
    key3 = true,
    key4 = var_name.output    // reference to prior step's output
  } timeout 30 retries 2 on_failure retry

  // Fire-and-forget (no binding, optional)
  call "notify" on "node" with {
    message = "done"
  } optional

  // Archive important mission values for downstream consumers.
  emit "result" kind answer value var_name.output
}
```

### Key rules

- **Dependencies are inferred from variable references** — write `input = photo.output` and the compiler builds the DAG automatically. No manual `depends_on`.
- **Phase partitioning** — Steps with no mutual dependencies execute in parallel (same phase). Steps that depend on prior steps execute in later phases.
- **Data flow** — Use `var.output` to pass results between steps across phases.
- **Emissions** — Use `emit "name" kind answer|context|evidence|diagnostic value <literal-or-var.output>` to append ordered mission archive records. Emits are not ability calls and do not affect the DAG.
- **Options**: `timeout <secs>`, `retries <n>`, `on_failure abort|skip|retry|continue`, `optional`

### Agent targets

Registered agents can be used as EAL targets. When `on "claude"` or `on "codex"` appears, the dispatcher routes to the agent CLI instead of a device.

```eal
mission "agent-review" {
  let analysis = call "review" on "claude" with {
    prompt = "Review this code for security issues"
  } timeout 120

  let perf = call "perf-check" on "codex" with {
    prompt = "Check for performance issues"
  } timeout 120

  // synthesis depends on both (phase 1)
  let report = call "synthesize" on "claude" with {
    prompt = "Create unified review report",
    security = analysis.output,
    performance = perf.output
  } timeout 180
}
```

For agent steps: the `prompt` argument is sent as the agent's input. Other arguments are included as context. The `function_name` (e.g., "review") becomes the task description.

### Register agents

```bash
easynet agent add claude --type claude-code --model sonnet
easynet agent add codex  --type codex --model gpt-5.2
```

### Run an EAL mission

```bash
# Compile only (inspect IR)
easynet mission run my-program.eal --emit-ir

# Execute
easynet mission run my-program.eal
```

## device.ability.json Schema

```json
{
  "name": "ability-name",
  "version": "1.0.0",
  "tool_name": "ability-name",
  "description": "What this ability does",
  "command": "shell command that produces JSON output"
}
```

**Required fields**: `name`, `command`
**Optional fields**: `version` (default "1.0.0"), `tool_name` (default = name), `description`

### Command patterns

The command should output JSON to stdout:

```json
// Simple value
"echo '{\"result\": \"hello\"}'"

// System info
"echo '{\"hostname\": \"'$(hostname)'\", \"os\": \"'$(uname -s)'\"}'"

// Run a script
"python3 /path/to/script.py"

// Pipe chain
"df -h / | tail -1 | awk '{print \"{\\\"used\\\": \\\"\" $3 \"\\\"}\"}"
```

## Patterns

### Device monitoring ability

```json
{
  "name": "health-check",
  "version": "1.0.0",
  "tool_name": "health-check",
  "description": "Quick health check: load, memory, disk",
  "command": "python3 -c \"import json,os; print(json.dumps({'load': os.getloadavg()[0], 'disk_free_gb': round(os.statvfs('/').f_bavail * os.statvfs('/').f_frsize / 1e9, 1)}))\""
}
```

### Multi-device pipeline (EAL)

```eal
mission "edge-inference" {
  let frame = call "capture" on "camera-01" with {
    resolution = "1080p"
  } timeout 10

  let result = call "inference" on "gpu-node" with {
    input = frame.output,
    model = "yolov8"
  } timeout 60 retries 1 on_failure retry

  call "store" on "nas" with {
    data = result.output,
    bucket = "detections"
  }
}
```

### Agent-powered ability (EAL)

```eal
mission "smart-deploy" {
  // Ask Claude to generate an ability for a task
  let ability_spec = call "design" on "claude" with {
    prompt = "Design an device.ability.json that monitors CPU temperature on Linux. Output valid JSON only."
  } timeout 60

  // Deploy the generated ability to a device
  let deployed = call "deploy_ability" on "device-01" with {
    tool_name = "cpu-temp",
    command = ability_spec.output,
    description = "Monitor CPU temperature"
  }
}
```

### Multi-agent discussion → article (EAL)

```eal
mission "collaborative-article" {
  let view_a = call "argue" on "claude" with {
    prompt = "Argue position A in 2 paragraphs"
  } timeout 120

  let view_b = call "argue" on "codex" with {
    prompt = "Argue position B in 2 paragraphs"
  } timeout 120

  let article = call "synthesize" on "claude" with {
    prompt = "Synthesize into a cohesive article",
    perspective_a = view_a.output,
    perspective_b = view_b.output
  } timeout 180
}
```

## Deploy lifecycle

```
device.ability.json  →  easynet deploy <dir> --to <node>
                      │
                      ├── Phase 1: Publish (register in Hub)
                      ├── Phase 2: Install (materialize on node)
                      └── Phase 3: Activate (enable invocation)
                      
                 easynet invoke <node> <ability>
                 easynet abilities --node <node>
```

## MCP integration

When running as an MCP server, agents can use these tools directly:

| Tool | Description |
|------|-------------|
| `deploy_ability` | Deploy ability to device |
| `invoke_ability` | Call deployed ability |
| `execute_command` | One-shot remote command |
| `run_mission` | Compile and execute EAL |
| `send_to_agent` | Dispatch to another agent (requires `--enable-agent-dispatch`) |

Install MCP for your agent:

```bash
easynet mcp-install claude
easynet mcp-install codex
```

## File locations

| Path | Purpose |
|------|---------|
| `~/.easynet/runtime.json` | Current runtime state |
| `~/.easynet/agents.json` | Registered agent configs |
| `~/.easynet/credentials.json` | Device credentials |
| `examples/*.eal` | EAL program examples |
| `examples/claude-skill/abilities/` | Pre-built ability templates |
