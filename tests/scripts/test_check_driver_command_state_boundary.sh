#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-driver-command-state-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

bash "$SCRIPT"

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p \
  "$SB/tools/scripts" \
  "$SB/src/daemon/execution/mission/drivers" \
  "$SB/src/daemon/execution/mission"

cp "$SCRIPT" "$SB/tools/scripts/check-driver-command-state-boundary.sh"

cat >"$SB/src/daemon/execution/mission/adapter.rs" <<'RS'
pub enum DriverCommand {
    Default,
    Explicit(String),
}

impl DriverCommand {
    pub fn from_registry_value(_: &str) -> Self {
        Self::Default
    }
}

pub struct InvokeOpts {
    pub command: DriverCommand,
}
RS

cat >"$SB/src/daemon/execution/mission/drivers/claude_code.rs" <<'RS'
pub struct ClaudeOptions {
    pub command: DriverCommand,
}

impl ClaudeOptions {
    pub fn resolved_command(&self) -> &str {
        self.command.resolve(DEFAULT_CLAUDE_BINARY)
    }
}
RS

cat >"$SB/src/daemon/execution/mission/drivers/codex.rs" <<'RS'
pub struct CodexOptions {
    pub command: DriverCommand,
}

impl CodexOptions {
    pub fn resolved_command(&self) -> &str {
        self.command.resolve(DEFAULT_CODEX_BINARY)
    }
}
RS

cat >"$SB/src/daemon/execution/mission/drivers/external.rs" <<'RS'
fn invoke(opts: InvokeOpts) {
    let _ = opts.command.explicit();
}
RS

cat >"$SB/src/daemon/execution/mission/dispatch.rs" <<'RS'
fn dispatch(entry: AgentEntry) {
    let _ = InvokeOpts {
        command: DriverCommand::from_registry_value(&entry.command),
    };
}
RS

( cd "$SB" && bash tools/scripts/check-driver-command-state-boundary.sh )

cat >>"$SB/src/daemon/execution/mission/drivers/external.rs" <<'RS'
fn reparses_registry_command(entry: AgentEntry) {
    let _ = DriverCommand::from_registry_value(&entry.command);
}
RS

if ( cd "$SB" && bash tools/scripts/check-driver-command-state-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected duplicate registry command bridge to fail"
fi

cat >"$SB/src/daemon/execution/mission/drivers/external.rs" <<'RS'
fn invoke(opts: InvokeOpts) {
    let _ = opts.command.explicit();
}
RS

cat >>"$SB/src/daemon/execution/mission/drivers/codex.rs" <<'RS'
pub struct BadOptions {
    pub command: String,
}
RS

if ( cd "$SB" && bash tools/scripts/check-driver-command-state-boundary.sh ) >/dev/null 2>&1; then
  fail "self-test expected string command sentinel to fail"
fi
