#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

run_audit() {
  local backend_root="$1"
  local easyremote_root="$2"
  python3 - "$backend_root" "$easyremote_root" <<'PY'
from __future__ import annotations

import sys
from pathlib import Path


def resolve_backend_root(candidate: Path) -> Path:
    candidate = candidate.resolve()
    if (candidate / "go.mod").exists():
        return candidate
    nested = candidate / "backend"
    if (nested / "go.mod").exists():
        return nested
    return candidate


backend = resolve_backend_root(Path(sys.argv[1]))
easyremote = Path(sys.argv[2]).resolve()
violations: list[str] = []


def require_file(root: Path, relative: str, label: str) -> str:
    path = root / relative
    if not path.is_file():
        violations.append(f"{label}:missing_file:{relative}")
        return ""
    return path.read_text(encoding="utf-8", errors="replace")


def require_contains(text: str, needle: str, label: str) -> None:
    if needle not in text:
        violations.append(f"{label}:missing:{needle}")


def forbid_contains(text: str, needle: str, label: str) -> None:
    if needle in text:
        violations.append(f"{label}:forbidden:{needle}")


def check_backend() -> None:
    if not backend.exists() or not (backend / "go.mod").is_file():
        violations.append(f"backend:missing_go_mod:{backend}")
        return

    legacy = backend / "internal/runtimeprofile"
    if legacy.exists():
        for path in legacy.rglob("*.go"):
            violations.append(
                "backend:legacy_runtimeprofile_source:"
                + path.relative_to(backend).as_posix()
            )

    directory = require_file(backend, "internal/sdkdirectory/directory.go", "backend:directory")
    for required in [
        "easynetsdk.NewRuntimeDirectoryProvider",
        "easynetsdk.NewDirectoryClient",
        "easynetsdk.NewRuntimeAbilityDescriptorProvider",
        "easynetsdk.NewAbilityDescriptorClient",
    ]:
        require_contains(directory, required, "backend:directory")
    for forbidden in [
        '"easynet-backend/internal/runtimeprofile"',
        'ability := "namespace.resolve"',
        "easynetsdk.NewInvocationBuilder",
    ]:
        forbid_contains(directory, forbidden, "backend:directory")

    receipt_adapter = require_file(backend, "internal/sdkreceipt/receipt.go", "backend:receipt")
    for required in [
        "*easynetsdk.ReceiptClient",
        "easynetsdk.ReceiptListRequest",
        "easynetsdk.ReceiptGetRequest",
        "easynetsdk.ReceiptTraceRequest",
        "c.receipts.List",
        "c.receipts.Get",
        "c.receipts.Trace",
    ]:
        require_contains(receipt_adapter, required, "backend:receipt")
    for forbidden in [
        '"invocation.history.list"',
        '"invocation.history.get"',
        '"invocation.trace.get"',
        "easynetsdk.NewInvocationBuilder",
        "c.ability.Invoke(ctx",
    ]:
        forbid_contains(receipt_adapter, forbidden, "backend:receipt")

    service_context = require_file(backend, "internal/svc/servicecontext.go", "backend:receipt_boot")
    for required in [
        "easynetsdk.NewRuntimeReceiptProvider",
        "easynetsdk.NewReceiptClient",
        "sdkreceipt.NewClient",
    ]:
        require_contains(service_context, required, "backend:receipt_boot")

    events = require_file(backend, "internal/sdkevents/events.go", "backend:events")
    for required in [
        "easynetsdk.NewRuntimeAbilityEventSubscriptionProvider",
        "easynetsdk.NewRuntimeEventSubscriptionClient",
        "c.events.Build",
        'metadata["backend_adapter"] = "sdkevents"',
    ]:
        require_contains(events, required, "backend:events")
    for forbidden in [
        '"easynet-backend/internal/runtimeprofile"',
        "easynetsdk.NewInvocationBuilder",
        "OwnerAbilityDescriptorRef",
    ]:
        forbid_contains(events, forbidden, "backend:events")

    admin = require_file(backend, "internal/sdkadmin/admin.go", "backend:admin")
    for required in [
        "easynetsdk.NewRuntimeAdminAbilityClient",
        "c.admin.ListSessions",
        "c.admin.RevokeDevice",
        'metadata["backend_adapter"] = "sdkadmin"',
    ]:
        require_contains(admin, required, "backend:admin")

    access = require_file(backend, "internal/sdkaccesscontrol/accesscontrol.go", "backend:access_control")
    for required in [
        "easynetsdk.NewRuntimeAccessControlProvider",
        "easynetsdk.NewAccessControlClient",
        "c.access.Grant",
        "c.access.Revoke",
        "c.access.List",
        "c.access.CreateRequest",
        "c.access.ResolveRequest",
        "c.access.ListRequests",
        "c.access.Explain",
        'metadata["backend_adapter"] = "sdkaccesscontrol"',
    ]:
        require_contains(access, required, "backend:access_control")
    for forbidden in [
        '"easynet-backend/internal/runtimeprofile"',
        "easynetsdk.NewInvocationBuilder",
        "func (c *Client) call",
        "c.ability.Invoke(ctx, call",
    ]:
        forbid_contains(access, forbidden, "backend:access_control")

    principal = require_file(backend, "internal/sdkprincipal/principal.go", "backend:principal")
    for required in [
        "easynetsdk.NewRuntimeAbilityClient",
        "easynetsdk.NewRuntimePrincipalProvider",
        "easynetsdk.NewPrincipalClient",
        "client.Create",
        "client.BindFirstKey",
        "client.IssueEnrollment",
        "client.IssueGrant",
        'metadata["backend_adapter"] = "sdkprincipal"',
    ]:
        require_contains(principal, required, "backend:principal")
    for forbidden in [
        '"easynet-backend/internal/runtimeprofile"',
        "easynetsdk.NewInvocationBuilder",
        "func (c *Client) call",
        '"principal.lifecycle.create"',
        '"principal.lifecycle.bind_first_key"',
        '"principal.lifecycle.issue_enrollment"',
        '"principal.lifecycle.issue_grant"',
        "c.ability.Invoke(ctx, call",
    ]:
        forbid_contains(principal, forbidden, "backend:principal")


def check_easyremote() -> None:
    if not easyremote.exists() or not (easyremote / "pyproject.toml").is_file():
        violations.append(f"easyremote:missing_pyproject:{easyremote}")
        return
    if (easyremote / "easyremote/_sdk_profiles.py").exists():
        violations.append("easyremote:retired_sdk_profiles_bridge_present")

    config = require_file(easyremote, "easyremote/config.py", "easyremote:config")
    for required in [
        "easynet_sdk.default_control_path().parent",
        "easynet_sdk.SdkEnvironment",
        "easynet_sdk.read_control_discovery",
    ]:
        require_contains(config, required, "easyremote:config")

    identity = require_file(easyremote, "easyremote/_sdk_identity.py", "easyremote:identity")
    for required in [
        "easynet_sdk.parse_ura",
        "easynet_sdk.device_ura",
        "easynet_sdk.agent_ura",
        "easynet_sdk.hub_ura",
        "easynet_sdk.resource_ura",
        "easynet_sdk.owner_ability_ura",
        "easynet_sdk.canonical_ability_descriptor_ref",
        "easynet_sdk.project_descriptor_ref",
    ]:
        require_contains(identity, required, "easyremote:identity")

    receipts = require_file(easyremote, "easyremote/receipts.py", "easyremote:receipt")
    for required in [
        "easynet_sdk.InvocationLifecycleState",
        "easynet_sdk.RuntimeReceipt.from_required_mapping",
        "easynet_sdk.ReceiptReference.from_runtime_receipt",
    ]:
        require_contains(receipts, required, "easyremote:receipt")
    for forbidden in [
        "class InvocationState",
        "def parse_receipt_hash",
    ]:
        forbid_contains(receipts, forbidden, "easyremote:receipt")

    transport = require_file(
        easyremote,
        "easyremote/_sdk_transport/__init__.py",
        "easyremote:transport",
    )
    for required in [
        "easynet_sdk.InvocationResultAdapter",
        "easynet_sdk.UnaryDispatchPool",
        "easynet_sdk.DaemonFrameStream",
        "easynet_sdk.DaemonLifecycleFacade",
    ]:
        require_contains(transport, required, "easyremote:transport")
    for forbidden in [
        "ctypes",
        "subprocess",
        "libeasynet_cli",
        "easynet_abi_",
    ]:
        forbid_contains(transport, forbidden, "easyremote:transport")

    client = require_file(easyremote, "easyremote/client.py", "easyremote:client")
    for required in [
        "easynet_sdk.StreamValueAdapter",
        "easynet_sdk.BidiSessionAdapter",
        "UnaryDispatchPool.connect",
    ]:
        require_contains(client, required, "easyremote:client")

    mission = require_file(easyremote, "easyremote/mission.py", "easyremote:mission")
    require_contains(mission, "easynet_sdk.ReceiptReference", "easyremote:mission")
    forbid_contains(mission, "namespace.resolve", "easyremote:mission")

    package = easyremote / "easyremote"
    for path in package.rglob("*.py"):
        if "__pycache__" in path.parts:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        if "namespace.resolve" in text:
            violations.append(
                "easyremote:product_local_directory_resolver:"
                + path.relative_to(easyremote).as_posix()
            )


check_backend()
check_easyremote()

if violations:
    print("downstream SDK consumer cutover violations:")
    for violation in violations:
        print(violation)
    raise SystemExit(1)

print("downstream SDK consumer cutover ok")
PY
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT

  good_backend="$tmp/EasyNet/backend"
  good_remote="$tmp/EasyRemote"
  mkdir -p \
    "$good_backend/internal/sdkdirectory" \
    "$good_backend/internal/sdkreceipt" \
    "$good_backend/internal/svc" \
    "$good_backend/internal/sdkevents" \
    "$good_backend/internal/sdkadmin" \
    "$good_backend/internal/sdkaccesscontrol" \
    "$good_backend/internal/sdkprincipal" \
    "$good_remote/easyremote/_sdk_transport"
  printf '%s\n' 'module easynet-backend' >"$good_backend/go.mod"
  cat >"$good_backend/internal/sdkdirectory/directory.go" <<'EOF'
package sdkdirectory

var _ = []string{
  "easynetsdk.NewRuntimeDirectoryProvider",
  "easynetsdk.NewDirectoryClient",
  "easynetsdk.NewRuntimeAbilityDescriptorProvider",
  "easynetsdk.NewAbilityDescriptorClient",
}
EOF
  cat >"$good_backend/internal/sdkreceipt/receipt.go" <<'EOF'
package sdkreceipt

var _ = []string{
  "*easynetsdk.ReceiptClient",
  "easynetsdk.ReceiptListRequest",
  "easynetsdk.ReceiptGetRequest",
  "easynetsdk.ReceiptTraceRequest",
  "c.receipts.List",
  "c.receipts.Get",
  "c.receipts.Trace",
}
EOF
  cat >"$good_backend/internal/svc/servicecontext.go" <<'EOF'
package svc

var _ = []string{"easynetsdk.NewRuntimeReceiptProvider", "easynetsdk.NewReceiptClient", "sdkreceipt.NewClient"}
EOF
  cat >"$good_backend/internal/sdkevents/events.go" <<'EOF'
package sdkevents

// metadata["backend_adapter"] = "sdkevents"
var _ = []string{"easynetsdk.NewRuntimeAbilityEventSubscriptionProvider", "easynetsdk.NewRuntimeEventSubscriptionClient", "c.events.Build", "metadata[\"backend_adapter\"] = \"sdkevents\""}
EOF
  cat >"$good_backend/internal/sdkadmin/admin.go" <<'EOF'
package sdkadmin

// metadata["backend_adapter"] = "sdkadmin"
var _ = []string{"easynetsdk.NewRuntimeAdminAbilityClient", "c.admin.ListSessions", "c.admin.RevokeDevice", "metadata[\"backend_adapter\"] = \"sdkadmin\""}
EOF
  cat >"$good_backend/internal/sdkaccesscontrol/accesscontrol.go" <<'EOF'
package sdkaccesscontrol

// metadata["backend_adapter"] = "sdkaccesscontrol"
var _ = []string{"easynetsdk.NewRuntimeAccessControlProvider", "easynetsdk.NewAccessControlClient", "c.access.Grant", "c.access.Revoke", "c.access.List", "c.access.CreateRequest", "c.access.ResolveRequest", "c.access.ListRequests", "c.access.Explain", "metadata[\"backend_adapter\"] = \"sdkaccesscontrol\""}
EOF
  cat >"$good_backend/internal/sdkprincipal/principal.go" <<'EOF'
package sdkprincipal

// metadata["backend_adapter"] = "sdkprincipal"
var _ = []string{"easynetsdk.NewRuntimeAbilityClient", "easynetsdk.NewRuntimePrincipalProvider", "easynetsdk.NewPrincipalClient", "client.Create", "client.BindFirstKey", "client.IssueEnrollment", "client.IssueGrant", "metadata[\"backend_adapter\"] = \"sdkprincipal\""}
EOF
  printf '%s\n' '[project]' 'name = "easyremote"' >"$good_remote/pyproject.toml"
  cat >"$good_remote/easyremote/config.py" <<'EOF'
import easynet_sdk

ROOT = easynet_sdk.default_control_path().parent
ENV = easynet_sdk.SdkEnvironment
READ = easynet_sdk.read_control_discovery
EOF
  cat >"$good_remote/easyremote/_sdk_identity.py" <<'EOF'
import easynet_sdk

_ = [
    easynet_sdk.parse_ura,
    easynet_sdk.device_ura,
    easynet_sdk.agent_ura,
    easynet_sdk.hub_ura,
    easynet_sdk.resource_ura,
    easynet_sdk.owner_ability_ura,
    easynet_sdk.canonical_ability_descriptor_ref,
    easynet_sdk.project_descriptor_ref,
]
EOF
  cat >"$good_remote/easyremote/receipts.py" <<'EOF'
import easynet_sdk

InvocationState = easynet_sdk.InvocationLifecycleState
RuntimeReceipt = easynet_sdk.RuntimeReceipt.from_required_mapping
ReceiptReference = easynet_sdk.ReceiptReference.from_runtime_receipt
EOF
  cat >"$good_remote/easyremote/_sdk_transport/__init__.py" <<'EOF'
import easynet_sdk

_ = [
    easynet_sdk.InvocationResultAdapter,
    easynet_sdk.UnaryDispatchPool,
    easynet_sdk.DaemonFrameStream,
    easynet_sdk.DaemonLifecycleFacade,
]
EOF
  cat >"$good_remote/easyremote/client.py" <<'EOF'
import easynet_sdk

_ = easynet_sdk.StreamValueAdapter
_ = easynet_sdk.BidiSessionAdapter
_ = "UnaryDispatchPool.connect"
EOF
  cat >"$good_remote/easyremote/mission.py" <<'EOF'
import easynet_sdk

_ = easynet_sdk.ReceiptReference
EOF
  run_audit "$good_backend" "$good_remote" >/dev/null

  mkdir -p "$good_backend/internal/runtimeprofile"
  cat >"$good_backend/internal/runtimeprofile/runtime.go" <<'EOF'
package runtimeprofile
EOF
  if run_audit "$good_backend" "$good_remote" >"$tmp/bad.out" 2>&1; then
    echo "self-test expected legacy runtimeprofile fixture to fail" >&2
    exit 1
  fi
  grep -Fq "legacy_runtimeprofile_source" "$tmp/bad.out"
  rm "$good_backend/internal/runtimeprofile/runtime.go"

  printf '%s\n' 'namespace.resolve' >>"$good_remote/easyremote/mission.py"
  if run_audit "$good_backend" "$good_remote" >"$tmp/bad2.out" 2>&1; then
    echo "self-test expected EasyRemote local directory resolver fixture to fail" >&2
    exit 1
  fi
  grep -Fq "easyremote:mission:forbidden:namespace.resolve" "$tmp/bad2.out"

  echo "check-downstream-sdk-consumer-cutover self-test ok"
  exit 0
fi

BACKEND_ROOT="${EASYNET_BACKEND_ROOT:-$REPO_ROOT/../EasyNet/backend}"
EASYREMOTE_ROOT="${EASYNET_EASYREMOTE_ROOT:-$REPO_ROOT/../EasyRemote}"
run_audit "${1:-$BACKEND_ROOT}" "${2:-$EASYREMOTE_ROOT}"
