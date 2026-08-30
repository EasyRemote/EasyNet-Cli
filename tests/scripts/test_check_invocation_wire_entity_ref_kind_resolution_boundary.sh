#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-invocation-wire-entity-ref-kind-resolution-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

bash "$SCRIPT"

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/daemon/invocation/dispatch"
cp "$SCRIPT" "$SB/tools/scripts/check-invocation-wire-entity-ref-kind-resolution-boundary.sh"

cat >"$SB/src/daemon/invocation/dispatch/invocation_wire.rs" <<'RS'
enum EntityRefKindResolution {
    CallableActor,
    Ability,
    Device,
    Resource,
    Session,
    Continuation,
    StateObject,
}

impl EntityRefKindResolution {
    fn from_ura(ura: &str) -> anyhow::Result<Self> {
        match crate::core::ura::parse_ura(ura.trim()).map(|parsed| parsed.kind) {
            Ok(crate::core::ura::URAKind::Agent) => Ok(Self::CallableActor),
            Ok(crate::core::ura::URAKind::Service) => Ok(Self::CallableActor),
            Ok(crate::core::ura::URAKind::Authority) => Ok(Self::CallableActor),
            Ok(crate::core::ura::URAKind::Device) => Ok(Self::Device),
            _ => anyhow::bail!("subject_ref_kind_unsupported:User"),
        }
    }

    fn protobuf_kind(self) -> EntityRefKind {
        match self {
            Self::CallableActor => EntityRefKind::Agent,
            Self::Device => EntityRefKind::Device,
            _ => EntityRefKind::Resource,
        }
    }
}

fn try_entity_ref(ura: String) {
    let kind = EntityRefKindResolution::from_ura(&ura)?.protobuf_kind();
}

fn top_level_subject_resolution(ura: &str) -> Option<EntityRefKindResolution> {
    Some(EntityRefKindResolution::Session)
}

fn user_subject_rejection_probe() {
    let _ = "subject_ref_kind_unsupported:User";
}
RS

( cd "$SB" && bash tools/scripts/check-invocation-wire-entity-ref-kind-resolution-boundary.sh )

cat >>"$SB/src/daemon/invocation/dispatch/invocation_wire.rs" <<'RS'
fn infer_entity_ref_kind() {}
RS

if ( cd "$SB" && bash tools/scripts/check-invocation-wire-entity-ref-kind-resolution-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected obsolete infer_entity_ref_kind helper to fail"
fi

python3 - "$SB/src/daemon/invocation/dispatch/invocation_wire.rs" <<'PY'
from pathlib import Path
path = Path(__import__("sys").argv[1])
text = path.read_text()
text = text.replace("fn infer_entity_ref_kind() {}\n", "")
text += "// subject kind fallback\n"
path.write_text(text)
PY

if ( cd "$SB" && bash tools/scripts/check-invocation-wire-entity-ref-kind-resolution-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected subject kind fallback vocabulary to fail"
fi
