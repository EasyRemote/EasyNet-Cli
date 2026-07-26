#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-start-ready-signer-proof-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/cli/commands" "$SB/src/daemon/boot/invocation"
cp "$SCRIPT" "$SB/tools/scripts/check-start-ready-signer-proof-boundary.sh"

cat >"$SB/src/cli/commands/start_boot_watcher.rs" <<'RS'
#[derive(Debug, Clone, Default)]
pub struct BootProgressOutcome {
    pub pages_port: Option<u16>,
    pub ready_capability_flags: Vec<String>,
}

impl BootProgressOutcome {
    pub fn has_ready_capability_flag(&self, flag: &str) -> bool {
        self.ready_capability_flags.iter().any(|candidate| candidate == flag)
    }
}

fn apply_ready(outcome: &mut BootProgressOutcome, disc: ControlDiscovery) {
    outcome.ready_capability_flags = disc.capability_flags.clone();
}
RS

cat >"$SB/src/cli/commands/start.rs" <<'RS'
fn run() -> anyhow::Result<()> {
    let boot = wait_for_daemon_boot()?;
    let mut daemon_handle = DaemonHandle;
    let attached_existing_daemon = false;
    validate_device_runtime_readiness(&boot, &creds)?;
    if !attached_existing_daemon {
        daemon_handle.stop()?;
    }
    save_runtime_projection_after_ready(&mut daemon_handle)?;
    console::style("Welcome,");
    Ok(())
}

fn validate_device_runtime_readiness(
    boot: &super::start_boot_watcher::BootProgressOutcome,
    creds: &Credentials,
) -> anyhow::Result<()> {
    let required = crate::daemon::control::discovery::flags::PAIRED_USER_RUNTIME_SIGNER;
    if !boot.has_ready_capability_flag(required) {
        anyhow::bail!("daemon Ready did not advertise runtime capability `{required}`")
    }
    let user_ura = creds.user_ura()?;
    KeyServiceRuntimeCallerSignerReadinessProbe.prove(&user_ura)?;
    Ok(())
}

trait RuntimeCallerSignerReadinessProbe {
    fn prove(&self, user_ura: &str) -> anyhow::Result<()>;
}

struct KeyServiceRuntimeCallerSignerReadinessProbe;

impl RuntimeCallerSignerReadinessProbe for KeyServiceRuntimeCallerSignerReadinessProbe {
    fn prove(&self, user_ura: &str) -> anyhow::Result<()> {
        crate::daemon::identity::self_identity::prove_runtime_caller_signer_custody(user_ura)?;
        Ok(())
    }
}

#[test]
fn start_runtime_readiness_accepts_paired_user_signer_custody() {}

#[test]
fn start_runtime_readiness_rejects_missing_paired_user_signer_flag() {}

#[test]
fn start_runtime_readiness_rejects_missing_credential_user_ura() {}

#[test]
fn start_runtime_readiness_rejects_failed_signer_custody_proof() {}
RS

cat >"$SB/src/daemon/boot/invocation/mod.rs" <<'RS'
fn boot() -> anyhow::Result<()> {
    register_paired_user_runtime_signer(&config, &trust_anchor_path, &trust_anchor_cell)?;
    ready_capability_flags.push(crate::daemon::control::discovery::flags::PAIRED_USER_RUNTIME_SIGNER.to_string());
    Ok(())
}

fn register_paired_user_runtime_signer(
    config: &DaemonConfig,
    trust_anchor_path: &PathBuf,
    trust_anchor_cell: &SharedTrustAnchor,
) -> anyhow::Result<()> {
    let client = crate::daemon::identity::self_identity::KeyringClient::default_path();
    let ensured = crate::daemon::identity::self_identity::ensure_user_runtime_signing_identity(
        &client, &user_ura,
    )?;
    let projection = ensured.projection;
    crate::daemon::identity::self_identity::prove_user_runtime_signing_projection_custody(
        &client,
        &user_ura,
        &projection,
    )?;
    RuntimeTrustContext {
        daemon_realm: config.realm().to_string(),
        trust_anchor_path: trust_anchor_path.clone(),
        cell: trust_anchor_cell.clone(),
    }
    .register_user_pubkey(user_ura.clone(), projection.public_key_b64.clone())?;
    Ok(())
}
RS

(
  cd "$SB"
  bash tools/scripts/check-start-ready-signer-proof-boundary.sh
) >/dev/null || fail "happy path should pass"

python3 - "$SB/src/cli/commands/start.rs" <<'PY'
import pathlib
path = pathlib.Path(__import__("sys").argv[1])
text = path.read_text()
text = text.replace("    validate_device_runtime_readiness(&boot, &creds)?;\n", "")
path.write_text(text)
PY

set +e
(
  cd "$SB"
  bash tools/scripts/check-start-ready-signer-proof-boundary.sh
) >/tmp/check-start-ready-signer-proof-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing ready signer validation should exit 1 (got $rc)"

python3 - "$SB/src/cli/commands/start.rs" <<'PY'
import pathlib
path = pathlib.Path(__import__("sys").argv[1])
text = path.read_text()
needle = "    if !attached_existing_daemon {\n"
if "validate_device_runtime_readiness(&boot, &creds)?" not in text:
    text = text.replace(needle, "    validate_device_runtime_readiness(&boot, &creds)?;\n" + needle)
path.write_text(text)
PY

python3 - "$SB/src/daemon/boot/invocation/mod.rs" <<'PY'
import pathlib
path = pathlib.Path(__import__("sys").argv[1])
text = path.read_text()
text = text.replace(
    "    crate::daemon::identity::self_identity::prove_user_runtime_signing_projection_custody(\n"
    "        &client,\n"
    "        &user_ura,\n"
    "        &projection,\n"
    "    )?;\n",
    "",
)
path.write_text(text)
PY

set +e
(
  cd "$SB"
  bash tools/scripts/check-start-ready-signer-proof-boundary.sh
) >/tmp/check-start-ready-signer-proof-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing daemon projection-bound proof should exit 1 (got $rc)"

echo "test_check_start_ready_signer_proof_boundary.sh: all cases passed"
