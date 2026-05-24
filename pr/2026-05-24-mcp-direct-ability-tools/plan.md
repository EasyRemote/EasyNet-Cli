# MCP Direct Ability Tools

## Goal

Make the EasyNet workspace MCP server expose callable abilities as first-class MCP tools for Codex and Claude Code, while preserving `<agent>.invoke` as a fallback/debug surface.

## Invariants

- EasyNet ability descriptors remain the source of truth for description, input schema, visibility, and canonical ability identity.
- MCP-specific naming stays inside the MCP profile adapter; core ability names and URAs are unchanged.
- Tool calls through advertised MCP names must dispatch to the same local ability registry path as canonical ability calls.
- Existing clients that call the canonical dotted MCP tool name must keep working.

## Verification

- Unit tests for MCP tool name projection and alias dispatch.
- Existing MCP profile tests.
- Targeted `cargo test --lib runtime::agents::profiles::mcp`.

## Outcome

- `tools/list` now advertises client-safe direct ability tools such as `openai_mcp_unit_converter__convert_length`.
- Each advertised tool carries the canonical EasyNet ability in metadata and dispatches back to that canonical name.
- Canonical dotted tool calls remain accepted as a legacy alias.
- `device.mcp.bridge.call_tool` uses the same advertised-name resolver as the stdio MCP provider, so the in-process bridge can invoke names returned by `device.mcp.bridge.list_tools`.
- Verified with:
  - `cargo test --lib runtime::agents::profiles::mcp`
  - `cargo test --lib runtime::agents::mcp_bridge_ability`
  - `cargo test --lib mcp`

## Metadata Extension

Goal: make MCP-projected abilities self-describing for both machines and LLM tool selection.

Invariants:

- Do not change the ability manifest TOML schema for the first cut.
- Keep canonical execution identity in `x-easynet.ability`; metadata is advisory, not dispatch input.
- Description annotations must be concise and deterministic so clients that only pass `description` to the model still expose owner and cost.
- Cost is static catalog metadata. Runtime token/API usage remains a separate invocation ledger concern.

Outcome:

- MCP-projected tool descriptions now start with `[EasyNet ability: ... | owner: ... | cost: ...]`.
- `x-easynet` now includes `owner_user`, `owner_agent`, `cost_kind`, and `cost_label`.
- Per-agent manifest-backed descriptors derive `exec_kind`; MCP exec descriptors also carry `mcp_server` and `mcp_tool` internally and classify Google/Maps-like upstreams as `external_metered`.
- Verified with:
  - `cargo test --lib runtime::agents::profiles::mcp`
  - `cargo test --lib mcp`
  - `cargo build --bin easynet`
  - isolated real `./target/debug/easynet mcp serve --agent openai` tools/list smoke
