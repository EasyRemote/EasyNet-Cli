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
        "easynetsdk.NewDirectoryClient",
        "easynetsdk.NewRuntimeAbilityDescriptorProvider",
        "easynetsdk.NewAbilityDescriptorClient",
    ]:
        require_contains(directory, required, "backend:directory")
    for forbidden in [
        '"easynet-backend/internal/runtimeprofile"',
        'ability := "namespace.resolve"',
        "easynetsdk.NewInvocationBuilder",
        "easynetsdk.NewRuntimeDirectoryProvider",
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

    service_context = require_file(backend, "internal/svc/servicecontext.go", "backend:profile_graph_boot")
    for required in [
        "newSDKProfileGraph(provider, backendIdentity)",
        "receipts = graph.Receipts",
        "principal = graph.Principal",
    ]:
        require_contains(service_context, required, "backend:profile_graph_boot")

    profile_graph = require_file(backend, "internal/svc/sdk_profile_graph.go", "backend:receipt_boot")
    for required in [
        "easynetsdk.NewRuntimeReceiptProvider",
        "easynetsdk.NewReceiptClient",
        "sdkreceipt.NewClient",
    ]:
        require_contains(profile_graph, required, "backend:receipt_boot")

    runtime_bridge = require_file(
        backend,
        "internal/sdktest/runtime_bridge.go",
        "backend:runtime_bridge_result_shape",
    )
    for required in [
        '"admission_receipt":',
        '"terminal_receipt":',
    ]:
        require_contains(runtime_bridge, required, "backend:runtime_bridge_result_shape")
    for forbidden in [
        '"receipt":',
    ]:
        forbid_contains(runtime_bridge, forbidden, "backend:runtime_bridge_result_shape")

    events = require_file(backend, "internal/sdkevents/events.go", "backend:events")
    for required in [
        "c.events.Build",
        "newRuntimeEventRouteCatalog",
        'metadata["backend_adapter"] = "sdkevents"',
    ]:
        require_contains(events, required, "backend:events")
    for forbidden in [
        '"easynet-backend/internal/runtimeprofile"',
        "eventcore.NewRouteCatalog",
        "eventcore.Route",
        "eventcore.CursorProjection",
        "eventcore.Topic",
        "easynetsdk.NewInvocationBuilder",
        "OwnerAbilityDescriptorRef",
        "NewRuntimeAbilityEventSubscriptionProvider",
        "NewRuntimeEventSubscriptionClient",
        "easynetsdk.NewRuntimeAbilityEventProvider",
        "easynetsdk.NewRuntimeEventDraftClient",
        "easynetprovider.RuntimeEventRoutes",
    ]:
        forbid_contains(events, forbidden, "backend:events")

    admin = require_file(backend, "internal/sdkadmin/admin.go", "backend:admin")
    for required in [
        "easynetsdk.NewRuntimeAdminAbilityClient",
        "easynetsdk.NewRuntimeAbilityClient",
        "c.admin.ListSessions",
        "c.ability.Invoke",
        '"federation.revoke"',
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

    user_key_registration = require_file(
        backend,
        "internal/logic/auth/register_user_pubkey.go",
        "backend:user_signing_key_principal",
    )
    for required in [
        "svcCtx.Principal.Get",
        "svcCtx.Principal.Create",
        "svcCtx.Principal.BindFirstKey",
        "svcCtx.Principal.AddKey",
        "PrincipalCarrierBase",
        "PrincipalProofBootstrap",
        "PrincipalProofActiveKey",
    ]:
        require_contains(user_key_registration, required, "backend:user_signing_key_principal")
    for forbidden in [
        "Identity.RegisterSigningKey",
        "identityprofile.SigningKeyRegistrationRequest",
        '"identity.register_pubkey"',
    ]:
        forbid_contains(user_key_registration, forbidden, "backend:user_signing_key_principal")

    user_key_flow_test = require_file(
        backend,
        "internal/logic/user/registerUserSigningKeyLogic_test.go",
        "backend:user_signing_key_sdk_flow",
    )
    for required in [
        "TestRegisterUserSigningKey_AccountFlowUsesRealSDKPrincipalAdapter",
        "principalprofile.NewClient",
        "easynetsdk.NewRuntimeClient",
        '"principal.lifecycle.get"',
        '"principal.lifecycle.create"',
        '"principal.lifecycle.bind_first_key"',
    ]:
        require_contains(user_key_flow_test, required, "backend:user_signing_key_sdk_flow")
    for forbidden in [
        "Identity.RegisterSigningKey",
        "identityprofile.SigningKeyRegistrationRequest",
        '"identity.register_pubkey"',
    ]:
        forbid_contains(user_key_flow_test, forbidden, "backend:user_signing_key_sdk_flow")

    http_user_key_flow_test = require_file(
        backend,
        "internal/handler/bridge_http_e2e_test.go",
        "backend:http_user_signing_key_sdk_flow",
    )
    for required in [
        "TestBridge_HTTP_E2E_RegisterPairTwoDevicesAndSignedTargetedInvoke",
        "registerHTTPSigningKey",
        '"/api/v1/user/me/signing-keys"',
        "newBridgeHTTPTestPrincipal",
        "principalprofile.NewClient",
        "easynetsdk.NewRuntimeClient",
        '"principal.lifecycle.get"',
        '"principal.lifecycle.create"',
        '"principal.lifecycle.bind_first_key"',
        "requirePrincipalAbilitySubsequence",
    ]:
        require_contains(http_user_key_flow_test, required, "backend:http_user_signing_key_sdk_flow")
    for forbidden in [
        "PrincipalFixture",
        "AddBinding(",
    ]:
        forbid_contains(http_user_key_flow_test, forbidden, "backend:http_user_signing_key_sdk_flow")

    user_key_list = require_file(
        backend,
        "internal/logic/auth/list_user_pubkeys.go",
        "backend:user_signing_key_list_principal",
    )
    for required in [
        "svcCtx.Principal.Get",
        "PrincipalCarrierBase",
        "PrincipalSnapshot",
        "PublicKeyBindingStateActive",
        "PublicKeyBindingStateRevoked",
        "PublicKeyBindingStateRotated",
    ]:
        require_contains(user_key_list, required, "backend:user_signing_key_list_principal")
    for forbidden in [
        "Identity.ListSigningKeys",
        "identityprofile.SigningKeyListRequest",
        '"identity.list_user_pubkeys"',
    ]:
        forbid_contains(user_key_list, forbidden, "backend:user_signing_key_list_principal")

    user_key_revoke = require_file(
        backend,
        "internal/logic/auth/revoke_user_pubkey.go",
        "backend:user_signing_key_revoke_principal",
    )
    for required in [
        "svcCtx.Principal.Get",
        "svcCtx.Principal.RevokeKey",
        "PrincipalCarrierBase",
        "RevokePrincipalKeyRequest",
        "PrincipalProofActiveKey",
    ]:
        require_contains(user_key_revoke, required, "backend:user_signing_key_revoke_principal")
    for forbidden in [
        "Identity.RevokeSigningKey",
        "identityprofile.SigningKeyRevokeRequest",
        '"identity.revoke_user_pubkey"',
    ]:
        forbid_contains(user_key_revoke, forbidden, "backend:user_signing_key_revoke_principal")


def check_easyremote() -> None:
    if not easyremote.exists() or not (easyremote / "pyproject.toml").is_file():
        violations.append(f"easyremote:missing_pyproject:{easyremote}")
        return
    if (easyremote / "easyremote/_sdk_profiles.py").exists():
        violations.append("easyremote:retired_sdk_profiles_bridge_present")

    config = require_file(easyremote, "easyremote/config.py", "easyremote:config")
    for required in [
        "easynet_sdk.runtime_state_root()",
        "easynet_sdk.SdkEnvironment",
        "easynet_sdk.read_runtime_control_discovery",
        "runtime_identity_projection",
        "sdk_environment().runtime_identity_projection",
    ]:
        require_contains(config, required, "easyremote:config")
    forbid_contains(config, "json.loads", "easyremote:config")

    local_identity = require_file(easyremote, "easyremote/identity.py", "easyremote:local_identity")
    for required in [
        "from_runtime_projection",
        "runtime_identity_projection()",
        "projection.runtime_instance_id",
    ]:
        require_contains(local_identity, required, "easyremote:local_identity")
    for forbidden in [
        "from .config import read_credentials",
        "read_credentials()",
    ]:
        forbid_contains(local_identity, forbidden, "easyremote:local_identity")

    retired_identity_bridge = easyremote / "easyremote/_sdk_identity.py"
    if retired_identity_bridge.exists():
        violations.append("easyremote:retired_sdk_identity_bridge_present")

    identity = require_file(easyremote, "easyremote/identity.py", "easyremote:identity")
    for required in [
        "easynet_sdk.parse_ura",
        "easynet_sdk.device_ura",
        "easynet_sdk.agent_ura",
        "easynet_sdk.authority_ura",
        "easynet_sdk.resource_ura",
    ]:
        require_contains(identity, required, "easyremote:identity")

    addressing = require_file(
        easyremote,
        "easyremote/_addressing.py",
        "easyremote:addressing",
    )
    for required in [
        "easynet_sdk.AddressingClient",
        "easynet_sdk.AxonAddressingTransport",
        "self._addressing.parse_ura",
        "self._addressing.device_ura",
        "self._addressing.owner_ability_ura",
        "self._addressing.project_ability_ura",
    ]:
        require_contains(addressing, required, "easyremote:addressing")

    invocation = require_file(
        easyremote,
        "easyremote/invocation.py",
        "easyremote:invocation_addressing",
    )
    require_contains(invocation, "easynet_sdk.InvocationDraft", "easyremote:invocation")
    for forbidden in [
        "class InvocationTuple",
        "InvocationWireProjector",
        "project_descriptor_ref",
        "def with_subject",
        "def with_causal",
    ]:
        forbid_contains(invocation, forbidden, "easyremote:invocation")

    if (easyremote / "easyremote/receipts.py").exists():
        violations.append("easyremote:canonical_receipt_model_present")
    context_dispatch = require_file(
        easyremote,
        "easyremote/_context_dispatch.py",
        "easyremote:receipt_reference",
    )
    require_contains(
        context_dispatch,
        "easynet_sdk.ReceiptReference.from_runtime_receipt",
        "easyremote:receipt_reference",
    )
    host_server = require_file(
        easyremote,
        "easyremote/_host/server.py",
        "easyremote:runtime_receipt",
    )
    require_contains(
        host_server,
        "easynet_sdk.RuntimeReceipt.from_required_mapping",
        "easyremote:runtime_receipt",
    )
    package_init = require_file(easyremote, "easyremote/__init__.py", "easyremote:exports")
    for forbidden in [
        '"InvocationTuple"',
        '"InvocationState"',
        '"Receipt"',
        '"ReceiptChain"',
    ]:
        forbid_contains(package_init, forbidden, "easyremote:exports")

    runtime_provider = require_file(
        easyremote,
        "easyremote/runtime_provider.py",
        "easyremote:runtime_provider",
    )
    for required in [
        "easynet_sdk.RuntimeConnection",
        "easynet_sdk.SdkEnvironment",
        "self._environment_factory().runtime_connection()",
    ]:
        require_contains(runtime_provider, required, "easyremote:runtime_provider")
    for forbidden in [
        "DaemonLifecycleFacade",
        "RuntimeLifecycle",
        ".daemon_control()",
        ".runtime_lifecycle()",
        "start_daemon",
        "start_runtime_host",
    ]:
        forbid_contains(runtime_provider, forbidden, "easyremote:runtime_provider")

    transport = require_file(
        easyremote,
        "easyremote/_sdk_transport/__init__.py",
        "easyremote:transport",
    )
    for required in [
        "easynet_sdk.InvocationResultAdapter",
        "easynet_sdk.UnaryDispatchPool",
        "easynet_sdk.RuntimeFrameStream",
        "easynet_sdk.RuntimeBidiChannel",
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
    require_contains(
        mission,
        "easynet_sdk.ReceiptReference.from_runtime_receipt",
        "easyremote:mission",
    )
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
    "$good_backend/internal/sdktest" \
    "$good_backend/internal/handler" \
    "$good_remote/easyremote/_sdk_transport"
  printf '%s\n' 'module easynet-backend' >"$good_backend/go.mod"
  cat >"$good_backend/internal/sdkdirectory/directory.go" <<'EOF'
package sdkdirectory

var _ = []string{
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

var _ = []string{"newSDKProfileGraph(provider, backendIdentity)", "receipts = graph.Receipts", "principal = graph.Principal"}
EOF
  cat >"$good_backend/internal/svc/sdk_profile_graph.go" <<'EOF'
package svc

var _ = []string{"easynetsdk.NewRuntimeReceiptProvider", "easynetsdk.NewReceiptClient", "sdkreceipt.NewClient"}
EOF
  cat >"$good_backend/internal/sdktest/runtime_bridge.go" <<'EOF'
package sdktest

var _ = map[string]any{
  "admission_receipt": nil,
  "terminal_receipt": nil,
}
EOF
  cat >"$good_backend/internal/sdkevents/events.go" <<'EOF'
package sdkevents

// metadata["backend_adapter"] = "sdkevents"
var _ = []string{"c.events.Build", "newRuntimeEventRouteCatalog", "metadata[\"backend_adapter\"] = \"sdkevents\""}
EOF
  cat >"$good_backend/internal/sdkadmin/admin.go" <<'EOF'
package sdkadmin

// metadata["backend_adapter"] = "sdkadmin"
var _ = []string{"easynetsdk.NewRuntimeAdminAbilityClient", "easynetsdk.NewRuntimeAbilityClient", "c.admin.ListSessions", "c.ability.Invoke", "federation.revoke", "metadata[\"backend_adapter\"] = \"sdkadmin\""}
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
  mkdir -p "$good_backend/internal/logic/auth"
  cat >"$good_backend/internal/logic/auth/register_user_pubkey.go" <<'EOF'
package auth

var _ = []string{"svcCtx.Principal.Get", "svcCtx.Principal.Create", "svcCtx.Principal.BindFirstKey", "svcCtx.Principal.AddKey", "PrincipalCarrierBase", "PrincipalProofBootstrap", "PrincipalProofActiveKey"}
EOF
  mkdir -p "$good_backend/internal/logic/user"
  cat >"$good_backend/internal/logic/user/registerUserSigningKeyLogic_test.go" <<'EOF'
package user

var _ = []string{
  "TestRegisterUserSigningKey_AccountFlowUsesRealSDKPrincipalAdapter",
  "principalprofile.NewClient",
  "easynetsdk.NewRuntimeClient",
  "principal.lifecycle.get",
  "principal.lifecycle.create",
  "principal.lifecycle.bind_first_key",
}
EOF
  cat >"$good_backend/internal/handler/bridge_http_e2e_test.go" <<'EOF'
package handler_test

var _ = []string{
  "TestBridge_HTTP_E2E_RegisterPairTwoDevicesAndSignedTargetedInvoke",
  "registerHTTPSigningKey",
  "/api/v1/user/me/signing-keys",
  "newBridgeHTTPTestPrincipal",
  "principalprofile.NewClient",
  "easynetsdk.NewRuntimeClient",
  "principal.lifecycle.get",
  "principal.lifecycle.create",
  "principal.lifecycle.bind_first_key",
  "requirePrincipalAbilitySubsequence",
}
EOF
  cat >"$good_backend/internal/logic/auth/list_user_pubkeys.go" <<'EOF'
package auth

var _ = []string{"svcCtx.Principal.Get", "PrincipalCarrierBase", "PrincipalSnapshot", "PublicKeyBindingStateActive", "PublicKeyBindingStateRevoked", "PublicKeyBindingStateRotated"}
EOF
  cat >"$good_backend/internal/logic/auth/revoke_user_pubkey.go" <<'EOF'
package auth

var _ = []string{"svcCtx.Principal.Get", "svcCtx.Principal.RevokeKey", "PrincipalCarrierBase", "RevokePrincipalKeyRequest", "PrincipalProofActiveKey"}
EOF
  printf '%s\n' '[project]' 'name = "easyremote"' >"$good_remote/pyproject.toml"
  cat >"$good_remote/easyremote/config.py" <<'EOF'
import easynet_sdk

ROOT = easynet_sdk.runtime_state_root()
ENV = easynet_sdk.SdkEnvironment
READ = easynet_sdk.read_runtime_control_discovery
PROJECTION = "runtime_identity_projection"
LOAD = "sdk_environment().runtime_identity_projection"
def runtime_identity_projection(): pass
def read_credentials(): pass
read_credentials = read_credentials
EOF
  cat >"$good_remote/easyremote/identity.py" <<'EOF'
import easynet_sdk
from .config import runtime_identity_projection

def from_runtime_projection(projection):
    return projection.runtime_instance_id

def load():
    return runtime_identity_projection()

_ = [
    easynet_sdk.parse_ura,
    easynet_sdk.device_ura,
    easynet_sdk.agent_ura,
    easynet_sdk.authority_ura,
    easynet_sdk.resource_ura,
]
EOF
  cat >"$good_remote/easyremote/_addressing.py" <<'EOF'
import easynet_sdk

class Resolver:
    def __init__(self):
        self._addressing = easynet_sdk.AddressingClient(
            easynet_sdk.AxonAddressingTransport()
        )

    def load(self):
        return [
            self._addressing.parse_ura,
            self._addressing.device_ura,
            self._addressing.owner_ability_ura,
            self._addressing.project_ability_ura,
        ]
EOF
  cat >"$good_remote/easyremote/invocation.py" <<'EOF'
import easynet_sdk

def prepare(draft: easynet_sdk.InvocationDraft):
    return draft
EOF
  cat >"$good_remote/easyremote/_context_dispatch.py" <<'EOF'
import easynet_sdk

_ = easynet_sdk.ReceiptReference.from_runtime_receipt
EOF
  mkdir -p "$good_remote/easyremote/_host"
  cat >"$good_remote/easyremote/_host/server.py" <<'EOF'
import easynet_sdk

_ = easynet_sdk.RuntimeReceipt.from_required_mapping
EOF
  cat >"$good_remote/easyremote/__init__.py" <<'EOF'
__all__ = []
EOF
  cat >"$good_remote/easyremote/runtime_provider.py" <<'EOF'
import easynet_sdk

class Provider:
    def __init__(self, environment_factory: type[easynet_sdk.SdkEnvironment]):
        self._environment_factory = environment_factory

    def connect(self) -> easynet_sdk.RuntimeConnection:
        return self._environment_factory().runtime_connection()
EOF
  cat >"$good_remote/easyremote/_sdk_transport/__init__.py" <<'EOF'
import easynet_sdk

_ = [
    easynet_sdk.InvocationResultAdapter,
    easynet_sdk.UnaryDispatchPool,
    easynet_sdk.RuntimeFrameStream,
    easynet_sdk.RuntimeBidiChannel,
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

_ = easynet_sdk.ReceiptReference.from_runtime_receipt
EOF
  run_audit "$good_backend" "$good_remote" >/dev/null

  printf '%s\n' '"""Retired identity compatibility bridge."""' >"$good_remote/easyremote/_sdk_identity.py"
  if run_audit "$good_backend" "$good_remote" >"$tmp/retired-identity.out" 2>&1; then
    echo "self-test expected retired EasyRemote SDK identity bridge to fail" >&2
    exit 1
  fi
  grep -Fq "easyremote:retired_sdk_identity_bridge_present" "$tmp/retired-identity.out"
  rm "$good_remote/easyremote/_sdk_identity.py"

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

  printf '%s\n' 'var _ = "Identity.RevokeSigningKey"' >>"$good_backend/internal/logic/auth/revoke_user_pubkey.go"
  if run_audit "$good_backend" "$good_remote" >"$tmp/bad3.out" 2>&1; then
    echo "self-test expected Backend legacy user signing-key revoke fixture to fail" >&2
    exit 1
  fi
  grep -Fq "backend:user_signing_key_revoke_principal:forbidden:Identity.RevokeSigningKey" "$tmp/bad3.out"

  printf '%s\n' 'var _ = "eventcore.NewRouteCatalog"' >>"$good_backend/internal/sdkevents/events.go"
  if run_audit "$good_backend" "$good_remote" >"$tmp/bad4.out" 2>&1; then
    echo "self-test expected Backend SDK route catalog fixture to fail" >&2
    exit 1
  fi
  grep -Fq "backend:events:forbidden:eventcore.NewRouteCatalog" "$tmp/bad4.out"

  printf '%s\n' 'var _ = map[string]any{"receipt": nil}' >>"$good_backend/internal/sdktest/runtime_bridge.go"
  if run_audit "$good_backend" "$good_remote" >"$tmp/bad5.out" 2>&1; then
    echo "self-test expected Backend retired runtime result receipt alias fixture to fail" >&2
    exit 1
  fi
  grep -Fq 'backend:runtime_bridge_result_shape:forbidden:"receipt":' "$tmp/bad5.out"

  echo "check-downstream-sdk-consumer-cutover self-test ok"
  exit 0
fi

BACKEND_ROOT="${EASYNET_BACKEND_ROOT:-$REPO_ROOT/../EasyNet/backend}"
EASYREMOTE_ROOT="${EASYNET_EASYREMOTE_ROOT:-$REPO_ROOT/../EasyRemote}"
run_audit "${1:-$BACKEND_ROOT}" "${2:-$EASYREMOTE_ROOT}"
