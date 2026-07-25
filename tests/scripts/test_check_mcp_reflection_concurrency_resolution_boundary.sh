#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-mcp-reflection-concurrency-resolution-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

bash "$SCRIPT"

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/daemon/ability/builtins/integrations/mcp"
cp "$SCRIPT" "$SB/tools/scripts/check-mcp-reflection-concurrency-resolution-boundary.sh"

cat >"$SB/src/daemon/ability/builtins/integrations/mcp/reflective_registry.rs" <<'RS'
const DEFAULT_MCP_REFLECTION_CONCURRENCY: usize = 4;

enum McpReflectionConcurrency {
    Configured(usize),
    Defaulted(McpReflectionConcurrencyDefaultReason),
}

enum McpReflectionConcurrencyDefaultReason {
    Missing,
    Empty,
    Invalid,
    NonPositive,
}

impl McpReflectionConcurrency {
    fn from_env() -> Self {
        Self::Defaulted(McpReflectionConcurrencyDefaultReason::Missing)
    }

    fn from_env_value(raw: Option<&str>) -> Self {
        Self::Configured(1)
    }

    fn limit(&self) -> usize {
        match self {
            Self::Configured(limit) => *limit,
            Self::Defaulted(_) => DEFAULT_MCP_REFLECTION_CONCURRENCY,
        }
    }
}

struct McpReflectionSupervisor {
    concurrency_limit: usize,
}

impl McpReflectionSupervisor {
    fn new() -> Self {
        Self {
            concurrency_limit: McpReflectionConcurrency::from_env().limit(),
        }
    }
}
RS

( cd "$SB" && bash tools/scripts/check-mcp-reflection-concurrency-resolution-boundary.sh )

cat >>"$SB/src/daemon/ability/builtins/integrations/mcp/reflective_registry.rs" <<'RS'
fn mcp_reflection_concurrency() -> usize {
    4
}
RS

if ( cd "$SB" && bash tools/scripts/check-mcp-reflection-concurrency-resolution-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected obsolete mcp_reflection_concurrency helper to fail"
fi

python3 - "$SB/src/daemon/ability/builtins/integrations/mcp/reflective_registry.rs" <<'PY'
from pathlib import Path
path = Path(__import__("sys").argv[1])
text = path.read_text()
text = text.replace("fn mcp_reflection_concurrency() -> usize {\n    4\n}\n", "")
text += "// malformed values fall back to DEFAULT_MCP_REFLECTION_CONCURRENCY\n"
path.write_text(text)
PY

if ( cd "$SB" && bash tools/scripts/check-mcp-reflection-concurrency-resolution-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected fallback vocabulary to fail"
fi
