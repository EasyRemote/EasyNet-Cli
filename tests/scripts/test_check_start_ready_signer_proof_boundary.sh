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

mkdir -p "$SB/tools/scripts" "$SB/src/cli/commands"
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

echo "test_check_start_ready_signer_proof_boundary.sh: all cases passed"
