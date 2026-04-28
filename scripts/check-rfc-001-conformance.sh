#!/usr/bin/env bash
# AXON-RFC-001 — Conformance script for EasyNet-Cli (P0 phase)
#
# Enforces architectural invariants of AXON-RFC-001 in the CLI repo.
# Source of truth: EasyNet-Axon/docs/rfc/AXON-RFC-001-invocation-as-sole-network-primitive.md
# Per-repo Delta Table: docs/rfc/AXON-RFC-001-cli-delta-table.md
#
# CLI-specific allowance:
#   MCP awareness is permitted ONLY inside the mcp-profile module
#   (src/runtime/agents/mcp.rs, planned for P4). All other locations
#   in src/ MUST be MCP-free.
#
# Phase = P0 (baseline mode).

set -euo pipefail

# Allow self-test to override the scan root via env var.
if [[ -n "${RFC001_FIXTURE_ROOT:-}" ]]; then
  ROOT="$RFC001_FIXTURE_ROOT"
else
  ROOT="$(cd "$(dirname "$0")/.." && pwd)"
fi
SRC="$ROOT/src"

PHASE="${RFC001_PHASE:-baseline}"
total_violations=0

count_pattern() {
  local label="$1"
  local pattern="$2"
  local scope="$3"
  shift 3
  local extra_excludes=()
  if [[ "$#" -gt 0 ]]; then
    extra_excludes=("$@")
  fi

  local paths=()
  for p in $scope; do
    [[ -e "$p" ]] && paths+=("$p")
  done
  if [[ "${#paths[@]}" -eq 0 ]]; then
    printf "  [skip] %-60s (no scope paths exist)\n" "$label"
    return
  fi

  local rg_args=("--pcre2" "-c" "--no-heading")
  rg_args+=("--glob" "!**/REMOVED-RFC-001*")
  rg_args+=("--glob" "!**/check-rfc-001-conformance.sh")
  rg_args+=("--glob" "!**/AXON-RFC-001*")
  if [[ "${#extra_excludes[@]}" -gt 0 ]]; then
    for excl in "${extra_excludes[@]}"; do
      rg_args+=("--glob" "!$excl")
    done
  fi

  local hits=0
  local raw
  raw="$(rg "${rg_args[@]}" "$pattern" "${paths[@]}" 2>/dev/null || true)"
  if [[ -n "$raw" ]]; then
    # rg -c output: "file:count" multi-file, or just "count" single-file.
    while IFS= read -r line; do
      if [[ "$line" == *":"* ]]; then
        local count="${line##*:}"
      else
        local count="$line"
      fi
      count="${count//[^0-9]/}"
      [[ -n "$count" ]] && hits=$((hits + count))
    done <<< "$raw"
  fi

  total_violations=$((total_violations + hits))

  if [[ "$hits" -eq 0 ]]; then
    printf "  [ ok ] %-60s 0\n" "$label"
  else
    printf "  [WARN] %-60s %d\n" "$label" "$hits"
    if [[ "$PHASE" == "enforce" ]]; then
      rg "${rg_args[@]/-c/-n}" "$pattern" "${paths[@]}" 2>/dev/null || true
    fi
  fi
}

echo "AXON-RFC-001 Conformance — EasyNet-Cli — phase=$PHASE"
echo "==================================================================="
echo

# ─────────────────────────────────────────────────────────────────
# Rule 1 (CLI side of proto surface): no calls to deleted axon RPCs.
# Note: most checks here are "code that calls deleted axon RPCs",
# since the proto itself lives in EasyNet-Axon.
# ─────────────────────────────────────────────────────────────────
echo "Rule 1 — No CLI call sites to deleted axon RPCs"

count_pattern "register_runtime_local_mcp_tool / unregister_*" \
  'register_runtime_local_mcp_tool|unregister_runtime_local_mcp_tool' \
  "$SRC"

count_pattern "publish_system_abilities (workaround landed in PR #5)" \
  'publish_system_abilities' \
  "$SRC"

count_pattern "system_skills_json (a2a label payload)" \
  'system_skills_json' \
  "$SRC"

echo

# ─────────────────────────────────────────────────────────────────
# Rule 2: Agent has no kind/role/type discriminator.
# ─────────────────────────────────────────────────────────────────
echo "Rule 2 — Agent has no kind/role/type discriminator"

count_pattern "enum AgentRole / AgentKind in CLI" \
  '\benum\s+(AgentRole|AgentKind)\b' \
  "$SRC"

count_pattern "agent.kind / agent.role branches" \
  '\bagent\.(kind|role)\b' \
  "$SRC"

# AgentType (claude-code | codex | codex-app-server) is the legacy
# Rust sub-agent type discriminator. Per the P4.4 decision it stays
# as an internal Rust type — the wire-level Agent envelope still has
# no `kind` field (Rule 2 above enforces that). The display string
# is surfaced through AbilityDescriptor.metadata["agent_type"] when
# the LLM-profile descriptor builder has it; see
# runtime/agents/profiles/llm.rs::descriptors_for_with_metadata.
# Counted (not [info]) so a future drop-the-internal-enum cleanup
# can use the count as its progress signal.
count_pattern "AgentType internal enum (kept by P4.4 decision; no wire impact)" \
  '\bAgentType\b' \
  "$SRC"

echo

# ─────────────────────────────────────────────────────────────────
# Rule 3: MCP only inside future src/runtime/agents/mcp.rs.
# Today (P0) that file does not exist — so any MCP appearance is
# legacy and counted against the baseline. After P4 lands the new
# module, the check excludes it via --glob.
# ─────────────────────────────────────────────────────────────────
echo "Rule 3 — MCP only inside mcp-profile module (P4)"

# MCP keyword scan. Allowed locations:
#   * runtime/agents/profiles/mcp.rs          — owns InvokeMcpProvider
#                                                 + tool spec projection
#                                                 (current production
#                                                 location).
#   * runtime/agents/mcp.rs                    — planned mcp-profile module
#                                                 path per RFC-001 plan.
#                                                 Kept in the allowlist so
#                                                 a future relocation does
#                                                 not require a script
#                                                 change; the self-test
#                                                 fixture also uses this
#                                                 path as its canonical
#                                                 example.
#   * facade/cli/mcp_server.rs                — `easynet mcp_server`
#                                                 user-facing edge entry
#   * facade/cli/start.rs                     — `easynet start --mcp`
#                                                 user-facing edge entry
#   * facade/mcp/mod.rs                        — quarantine anchor (doc only)
count_pattern "MCP keyword in CLI src (case-insensitive)" \
  '(?i)\bmcp\b' \
  "$SRC" \
  "**/runtime/agents/profiles/mcp.rs" \
  "**/runtime/agents/mcp.rs" \
  "**/facade/cli/mcp_server.rs" \
  "**/facade/cli/start.rs" \
  "**/facade/mcp/mod.rs"

# P4.8d quarantine check: facade/mcp may exist iff
#   (a) it contains only mod.rs (no submodule files), AND
#   (b) mod.rs imports nothing — it's a pure documentation anchor.
# Independent dispatch / tool catalog / handlers in this directory
# are violations of plan §A3 + §A5.
if [[ -d "$SRC/facade/mcp" ]]; then
  # Mere presence of the legacy directory is itself worth flagging so a
  # future RFC-001 P4.8d cleanup ("delete facade/mcp/") has a tracking
  # signal even when the directory is currently quarantine-clean. The
  # presence WARN does not increment total_violations on its own — it is
  # an informational tracker. Submodule files or imports inside the dir
  # are real violations and DO bump the count below.
  printf "  [WARN] %-60s exists\n" "facade/mcp/ legacy directory (P4.8d removal pending)"

  unexpected_files=$(find "$SRC/facade/mcp" -type f -name '*.rs' ! -name 'mod.rs' 2>/dev/null)
  if [[ -n "$unexpected_files" ]]; then
    printf "  [WARN] %-60s present\n" "facade/mcp/ submodule files (must be in mcp-profile)"
    total_violations=$((total_violations + 1))
    if [[ "$PHASE" == "enforce" ]]; then
      echo "    Unexpected files:"
      while IFS= read -r f; do echo "      $f"; done <<< "$unexpected_files"
    fi
  else
    printf "  [ ok ] %-60s quarantine anchor only\n" "facade/mcp/ contains only mod.rs"
  fi
  if [[ -f "$SRC/facade/mcp/mod.rs" ]] && grep -qE "^use|extern crate" "$SRC/facade/mcp/mod.rs" 2>/dev/null; then
    printf "  [WARN] %-60s present\n" "facade/mcp/mod.rs imports (must be doc-only)"
    total_violations=$((total_violations + 1))
  elif [[ -f "$SRC/facade/mcp/mod.rs" ]]; then
    printf "  [ ok ] %-60s no imports\n" "facade/mcp/mod.rs is doc-only"
  fi
else
  printf "  [ ok ] %-60s absent\n" "facade/mcp/ directory"
fi

echo

# ─────────────────────────────────────────────────────────────────
# Rule 4: system.* ability namespace must be retired.
# ─────────────────────────────────────────────────────────────────
echo "Rule 4 — system.* ability namespace retired"

count_pattern "system.skill.* / system.session.* / system.permission.* / etc." \
  'system\.(skill|session|permission|discuss|schedule|loop|memory|ping)\b' \
  "$SRC"

if [[ -d "$SRC/runtime/system" ]]; then
  printf "  [WARN] %-60s exists\n" "src/runtime/system/ directory presence (delete in P4)"
  total_violations=$((total_violations + 1))
else
  printf "  [ ok ] %-60s absent\n" "src/runtime/system/ directory presence (delete in P4)"
fi

echo

# ─────────────────────────────────────────────────────────────────
# Rule 5: No legacy SSH / exec compat path.
# ─────────────────────────────────────────────────────────────────
echo "Rule 5 — No legacy SSH/exec compatibility env var"

count_pattern "EASYNET_SESSION_BRIDGE_EXEC_ENABLED" \
  'EASYNET_SESSION_BRIDGE_EXEC_ENABLED' \
  "$SRC"

count_pattern "session_bridge legacy tool name" \
  'session_bridge' \
  "$SRC"

echo

# ─────────────────────────────────────────────────────────────────
# Rule 6: §A12 — hosted Agent receipts include signer/callee/host_attestation.
# Deferred to P4 when receipt schema lands.
# ─────────────────────────────────────────────────────────────────
echo "Rule 6 — Hosted Agent receipt schema (deferred to P4)"
echo "  [info] receipt schema lands in P4"

# ─────────────────────────────────────────────────────────────────
# Rule 7: §A13 — hosted Agent URI persistence.
# Deferred to P4 when local-agents.json lands.
# ─────────────────────────────────────────────────────────────────
echo "Rule 7 — local-agents.json hosted URI persistence (deferred to P4)"
echo "  [info] persistence lands in P4"

# ─────────────────────────────────────────────────────────────────
# Rule 8: §A6 — admission_internal kernel-local only.
# Deferred to P3 when admission gate lands.
# ─────────────────────────────────────────────────────────────────
echo "Rule 8 — admission_internal kernel-local (deferred to P3)"

count_pattern "admission_internal accepted from external (heuristic)" \
  '\badmission_internal\s*=' \
  "$SRC"

echo

# ─────────────────────────────────────────────────────────────────
# Rule 9: §A14 — DelegationProof schema present.
# Deferred to P3.
# ─────────────────────────────────────────────────────────────────
echo "Rule 9 — DelegationProof schema (deferred to P3)"
echo "  [info] DelegationProof schema lands in P3"

# ─────────────────────────────────────────────────────────────────
# Rule 10: §A15 — AbilityDescriptor with scope metadata.
# Deferred to P4.
# ─────────────────────────────────────────────────────────────────
echo "Rule 10 — AbilityDescriptor scope metadata (deferred to P4)"
echo "  [info] AbilityDescriptor lands in P4"

echo

# ─────────────────────────────────────────────────────────────────
# Summary
# ─────────────────────────────────────────────────────────────────
echo "==================================================================="
echo "Total flagged occurrences: $total_violations"
echo

if [[ "$PHASE" == "baseline" ]]; then
  echo "Phase: baseline (P0). Counts recorded; CI does not fail."
  echo "Set RFC001_PHASE=enforce to fail on any non-zero count."
  exit 0
fi

if [[ "$total_violations" -gt 0 ]]; then
  echo "Phase: enforce. RFC-001 conformance violations detected."
  exit 1
fi

echo "Phase: enforce. All conformance rules satisfied."
exit 0
