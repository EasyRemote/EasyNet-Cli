#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-mcp-cost-metadata-projection-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

bash "$SCRIPT"

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/daemon/ability/catalog/profiles"
cp "$SCRIPT" "$SB/tools/scripts/check-mcp-cost-metadata-projection-boundary.sh"

cat >"$SB/src/daemon/ability/catalog/profiles/mcp.rs" <<'RS'
enum CostMetadataProjection {
    Declared { kind: String, label: String },
    UndeclaredKnownLlm,
    Undeclared,
}

impl CostMetadataProjection {
    fn from_descriptor(descriptor: &AbilityDescriptor) -> Self {
        Self::Undeclared
    }

    fn kind(&self) -> &str {
        match self {
            Self::Declared { kind, .. } => kind.as_str(),
            Self::UndeclaredKnownLlm => "llm_metered",
            Self::Undeclared => "unknown",
        }
    }

    fn label(&self) -> &str {
        match self {
            Self::Declared { label, .. } => label.as_str(),
            Self::UndeclaredKnownLlm => "LLM token billing may apply",
            Self::Undeclared => "cost not declared",
        }
    }
}

fn tool_spec_from_descriptor_with_name(descriptor: &AbilityDescriptor) {
    let cost = CostMetadataProjection::from_descriptor(descriptor);
    let _ = json!({
        "cost_kind": cost.kind(),
        "cost_label": cost.label(),
    });
}
RS

( cd "$SB" && bash tools/scripts/check-mcp-cost-metadata-projection-boundary.sh )

cat >>"$SB/src/daemon/ability/catalog/profiles/mcp.rs" <<'RS'
fn inferred_cost_kind() -> &'static str {
    "free"
}
RS

if ( cd "$SB" && bash tools/scripts/check-mcp-cost-metadata-projection-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected inferred cost helper to fail"
fi

