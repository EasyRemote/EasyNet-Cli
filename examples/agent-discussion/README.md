# EasyNet Agent Discussion — Multi-Agent Collaborative Writing

Demonstrates EasyNet's **reverse agent invocation**: dispatching tasks TO Claude Code
and Codex, orchestrating a multi-round discussion, and synthesizing the output into
a cohesive article.

## Architecture

```
┌─────────────────────────────────────┐
│  easynet discuss                    │
│  (conversation orchestrator)        │
│  - manages rounds + context         │
│  - formats prompts per agent        │
│  - collects and merges responses    │
└──────────┬──────────────┬───────────┘
           │              │
           ▼              ▼
┌──────────────────┐  ┌──────────────────┐
│  Claude Code     │  │  Codex           │
│  claude -p       │  │  codex exec      │
│  (stdin → stdout)│  │  (stdin → stdout)│
└──────────────────┘  └──────────────────┘
```

## Quick Start

```bash
# 1. Build easynet
cargo build --release

# 2. Register agents
easynet agent add claude --type claude-code --model sonnet
easynet agent add codex  --type codex --model gpt-5.2

# 3. Check availability
easynet agent doctor

# 4. Run the discussion
./examples/agent-discussion/run.sh

# Or manually:
easynet discuss \
  --agents claude,codex \
  --rounds 3 \
  --topic "$(cat examples/agent-discussion/topic-alive.txt)" \
  --output alive-article.md
```

## What Happens

1. **Round 1**: Each agent presents their initial perspective on the topic
2. **Round 2**: Each agent reads all prior responses and builds on them — challenging,
   extending, or synthesizing ideas
3. **Round 3 (final)**: The first agent synthesizes everything into a cohesive article,
   the second adds concluding thoughts

Each agent sees the full conversation history (with automatic truncation of older
rounds to stay within context limits).

## Single Agent Dispatch

Test individual agent invocation:

```bash
easynet agent send claude "What is the most important unsolved problem in distributed AI?"
easynet agent send codex  "Explain agent-native networking in 3 sentences."
```

## Agent Types

| Type | CLI | Mode |
|------|-----|------|
| `claude-code` | `claude -p` | Print mode, stdin prompt, text output |
| `codex` | `codex exec` | Non-interactive exec, stdin prompt |
| `codex-app-server` | `codex app-server` | JSON-RPC 2.0 with threads and streaming |

## MCP Agent-to-Agent (Advanced)

When running easynet as an MCP server with agent dispatch enabled, agents can invoke
each other through the `send_to_agent` tool:

```bash
easynet mcp-server --enable-agent-dispatch
```

This enables circular agent-to-agent communication through EasyNet Hub.

## Files

```
agent-discussion/
├── README.md           ← This file
├── run.sh              ← Full demo script
└── topic-alive.txt     ← Discussion topic prompt
```
