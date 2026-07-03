#!/usr/bin/env bash
# mcp_bench_setup.sh — One-shot operator setup for running MCP-Bench
# (Accenture/mcp-bench) on top of an EasyNet Cli daemon.
#
# What it does
# ------------
#   1. Clones https://github.com/Accenture/mcp-bench into
#      ~/.easynet/vendor/mcp-bench (idempotent: pulls if present).
#   2. Runs mcp-bench's own mcp_servers/packaging/release/install.sh — that script
#      clones+builds the 28 upstream MCP servers under
#      ~/.easynet/vendor/mcp-bench/mcp_servers/<server-name>/.
#   3. Translates mcp_servers/commands.json into
#      ~/.easynet/mcp_clients.json — the schema McpClientService
#      reads at daemon boot. The translation:
#         * stdio entries (`cmd` only) → {name, command, args, env,
#           transport: "stdio"}
#         * the one HTTP entry (Google Maps, transport: "http") →
#           {name, command, args, env, transport: "http",
#           url: "http://127.0.0.1:<port>", endpoint: "<endpoint>"}
#         * `cwd` is rebased to the absolute mcp-bench path so the
#           daemon can spawn from any working directory.
#         * env list is preserved verbatim; the operator is
#           responsible for exporting the named variables before
#           starting the daemon (mcp-bench documents the keys
#           required in its config/api_key/ directory).
#
# Once this script finishes:
#
#   * Start the daemon:           easynet runtime start --foreground
#     The boot path calls McpClientService::from_path on
#     mcp_clients.json + the reflective registry exposes every tool
#     under each server as an EasyNet ability.
#   * Verify reflection:          easynet abilities --format json |
#                                   jq '[.[] | select(.source |
#                                     startswith("mcp_upstream:"))] |
#                                     length'
#     A green run shows ~250 abilities (28 servers × avg 9 tools).
#   * Run mcp-bench against EasyNet's mega-server: see task #9
#     (A4 in the plan) for the bench harness wiring.
#
# Why this script is here, not in mcp-bench
# -----------------------------------------
# Translating commands.json into mcp_clients.json is EasyNet-side
# bookkeeping. mcp-bench has its own loader (Python, via
# `mcp_modules/server_manager.py`) that consumes commands.json
# directly; that's fine, but it doesn't know about EasyNet's
# reflective ability surface. This script is the bridge so that
# both worlds share the same upstream server definitions without
# the operator hand-keying 28 entries.
#
# Idempotency
# -----------
# Safe to re-run. Existing checkouts are pulled; existing
# mcp_clients.json is backed up to .bak then overwritten. Missing
# python3 / jq / git surfaces a clear error rather than half-done
# state.
set -euo pipefail

# ── Tunables ────────────────────────────────────────────────────
MCP_BENCH_REPO="${MCP_BENCH_REPO:-https://github.com/Accenture/mcp-bench.git}"
MCP_BENCH_DIR="${MCP_BENCH_DIR:-$HOME/.easynet/vendor/mcp-bench}"
MCP_CLIENTS_JSON="${MCP_CLIENTS_JSON:-$HOME/.easynet/mcp_clients.json}"
SKIP_INSTALL_SH="${SKIP_INSTALL_SH:-0}"  # set to 1 to skip the 28-server install

# ── Pre-flight ──────────────────────────────────────────────────
need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "ERROR: required tool not found: $1" >&2
    exit 1
  }
}
need git
need python3
need jq

mkdir -p "$(dirname "$MCP_BENCH_DIR")"
mkdir -p "$(dirname "$MCP_CLIENTS_JSON")"

# ── 1. Clone or pull mcp-bench ──────────────────────────────────
if [[ -d "$MCP_BENCH_DIR/.git" ]]; then
  echo "[mcp_bench_setup] updating existing checkout: $MCP_BENCH_DIR"
  git -C "$MCP_BENCH_DIR" pull --ff-only
else
  echo "[mcp_bench_setup] cloning $MCP_BENCH_REPO → $MCP_BENCH_DIR"
  git clone "$MCP_BENCH_REPO" "$MCP_BENCH_DIR"
fi

COMMANDS_JSON="$MCP_BENCH_DIR/mcp_servers/commands.json"
if [[ ! -f "$COMMANDS_JSON" ]]; then
  echo "ERROR: $COMMANDS_JSON missing — mcp-bench layout may have changed" >&2
  exit 1
fi

# ── 2. Optionally run mcp-bench's installer ─────────────────────
INSTALL_SH="$MCP_BENCH_DIR/mcp_servers/packaging/release/install.sh"
if [[ "$SKIP_INSTALL_SH" == "1" ]]; then
  echo "[mcp_bench_setup] SKIP_INSTALL_SH=1 — skipping 28-server install"
elif [[ -x "$INSTALL_SH" ]]; then
  echo "[mcp_bench_setup] running mcp-bench packaging/release/install.sh (this can take a while)"
  ( cd "$MCP_BENCH_DIR/mcp_servers" && ./packaging/release/install.sh )
else
  echo "WARN: $INSTALL_SH not executable or absent — skipping; you'll need"
  echo "      to install the 28 upstream servers manually before booting"
  echo "      easynet (otherwise reflective registration will log failures"
  echo "      per server, then continue)."
fi

# ── 3. Translate commands.json → mcp_clients.json ───────────────
if [[ -f "$MCP_CLIENTS_JSON" ]]; then
  cp "$MCP_CLIENTS_JSON" "${MCP_CLIENTS_JSON}.bak"
  echo "[mcp_bench_setup] backed up existing config to ${MCP_CLIENTS_JSON}.bak"
fi

# Use a python heredoc rather than jq so we can split shell-quoted
# `cmd` into command + args (jq has no shell-tokenizer) and
# absolutise `cwd` against the mcp-bench layout. python3 is
# already a hard dependency.
python3 - <<PY > "$MCP_CLIENTS_JSON"
import json, os, shlex, sys, pathlib
COMMANDS_JSON = "$COMMANDS_JSON"
MCP_BENCH_DIR = pathlib.Path("$MCP_BENCH_DIR").resolve()
SERVERS_DIR   = MCP_BENCH_DIR / "mcp_servers"

with open(COMMANDS_JSON) as fh:
    commands = json.load(fh)

servers = []
for name, entry in commands.items():
    cmd_str = entry.get("cmd", "")
    parts = shlex.split(cmd_str) if cmd_str else []
    command = parts[0] if parts else ""
    args = parts[1:] if len(parts) > 1 else []

    # mcp-bench's commands.json records cwd values such as
    # "../wikipedia-mcp", while packaging/release/install.sh places the checkout at
    # mcp_servers/wikipedia-mcp. Prefer the real installed path
    # when present, but keep a deterministic fallback for dry-run
    # translations before packaging/release/install.sh has run.
    cwd_rel = entry.get("cwd")
    if cwd_rel:
        candidates = [SERVERS_DIR / cwd_rel]
        if cwd_rel.startswith("../"):
            candidates.insert(0, SERVERS_DIR / cwd_rel[3:])
        cwd_abs = next((p.resolve() for p in candidates if p.exists()), candidates[0].resolve())
    else:
        cwd_abs = None

    # If the upstream has its own uv venv (per-server isolation
    # via uv sync or uv venv), wrap the command in uv run so the
    # child uses the venv interpreter + dependencies. Without this,
    # a bare python inherits the daemon PATH (often the host system
    # or anaconda python), where the upstream fastmcp etc. are NOT
    # installed.
    #
    # Detection: presence of cwd/.venv/ is the standard uv marker.
    # If the cmd_str already starts with uv run (explicit operator
    # choice) or node (JS upstream), do not touch it.
    cmd_already_uv = cmd_str.lstrip().startswith("uv run")
    cmd_is_node    = cmd_str.lstrip().startswith("node ")
    venv_present   = cwd_abs is not None and (cwd_abs / ".venv").exists()
    if cwd_abs and venv_present and not cmd_already_uv and not cmd_is_node:
        effective_cmd = f"uv run {cmd_str}"
    else:
        effective_cmd = cmd_str

    # Wrap command so the child inherits cwd. The McpServerSpec
    # schema doesn't yet carry a 'cwd' field; standard POSIX
    # workaround: prefix with 'sh -c "cd <cwd> && exec <cmd>"'.
    if cwd_abs:
        full = f"cd {shlex.quote(str(cwd_abs))} && exec {effective_cmd}"
        wrapped_command = "sh"
        wrapped_args = ["-c", full]
    else:
        wrapped_command = command
        wrapped_args = args

    # env in commands.json is a list of NAMES the upstream needs;
    # we forward whatever the operator has in their shell at boot.
    env_names = entry.get("env", []) or []
    env_dict = {}
    for k in env_names:
        v = os.environ.get(k)
        if v is not None:
            env_dict[k] = v

    transport = entry.get("transport", "stdio")
    spec = {
        "name": name,
        "command": wrapped_command,
        "args": wrapped_args,
        "env": env_dict,
        "transport": transport,
    }
    if transport == "http":
        port = entry.get("port", 3001)
        endpoint = entry.get("endpoint", "/mcp")
        spec["url"] = f"http://127.0.0.1:{port}"
        spec["endpoint"] = endpoint

    servers.append(spec)

json.dump({"servers": servers}, sys.stdout, indent=2)
sys.stdout.write("\n")
PY

# Sanity: jq parse the resulting file so a malformed write would
# trip immediately, before the operator boots the daemon and gets
# a confusing parse error from McpClientService::from_path.
SERVER_COUNT=$(jq '.servers | length' "$MCP_CLIENTS_JSON")
HTTP_COUNT=$(jq '[.servers[] | select(.transport == "http")] | length' "$MCP_CLIENTS_JSON")
STDIO_COUNT=$(jq '[.servers[] | select(.transport == "stdio")] | length' "$MCP_CLIENTS_JSON")

echo
echo "[mcp_bench_setup] wrote $SERVER_COUNT server specs to $MCP_CLIENTS_JSON"
echo "  stdio: $STDIO_COUNT"
echo "  http:  $HTTP_COUNT"
echo
echo "Next steps:"
echo "  1. Export any API keys mcp-bench upstreams need (see"
echo "     $MCP_BENCH_DIR/config/api_key/ for the list)."
echo "  2. easynet runtime start --foreground"
echo "  3. easynet abilities --format json | jq '[.[] | select(.source |"
echo "       startswith(\"mcp_upstream:\"))] | length' # → should be ~250"
