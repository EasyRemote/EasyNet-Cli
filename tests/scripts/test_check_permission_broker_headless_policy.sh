#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-permission-broker-headless-policy.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

bash "$SCRIPT"

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p \
  "$SB/tools/scripts" \
  "$SB/src/daemon/execution/permission" \
  "$SB/src/daemon/boot/kernel" \
  "$SB/src/daemon/ability/builtins/governance" \
  "$SB/src/bin"

cp "$SCRIPT" "$SB/tools/scripts/check-permission-broker-headless-policy.sh"

cat >"$SB/src/daemon/execution/permission/mod.rs" <<'RS'
pub trait PermissionBroker {
    fn ask(&self);
}

pub enum UnobservedPermissionPolicy {
    Allow,
}

impl UnobservedPermissionPolicy {
    fn decide(self) {}
}

pub struct HeadlessPermissionBroker {
    unobserved_policy: UnobservedPermissionPolicy,
}

pub struct SubscriberBroker {
    unobserved_policy: UnobservedPermissionPolicy,
}

impl SubscriberBroker {
    pub fn ask(&self) {
        if !self.has_subscribers() {
            return self.unobserved_policy.decide();
        }
    }

    fn has_subscribers(&self) -> bool {
        false
    }
}

pub struct PermissionService;

impl PermissionService {
    pub fn headless() -> Self {
        Self
    }

    pub fn interactive() -> Self {
        Self
    }
}
RS

cat >"$SB/src/daemon/boot/kernel/mod.rs" <<'RS'
pub struct Kernel;

impl Kernel {
    pub fn new_interactive() -> Self {
        let _ = PermissionService::interactive();
        Self
    }
}
RS

cat >"$SB/src/daemon/ability/builtins/governance/consent.rs" <<'RS'
pub fn subscribe_handler() {}
RS

cat >"$SB/src/bin/easynet-daemon.rs" <<'RS'
fn main() {
    let _ = Kernel::new_interactive();
}
RS

( cd "$SB" && bash tools/scripts/check-permission-broker-headless-policy.sh )

printf '\npub struct AllowAllBroker;\n' >>"$SB/src/daemon/execution/permission/mod.rs"
if ( cd "$SB" && bash tools/scripts/check-permission-broker-headless-policy.sh ) >/dev/null 2>&1; then
  fail "self-test expected retired AllowAllBroker vocabulary to fail"
fi
