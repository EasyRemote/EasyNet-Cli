from __future__ import annotations

import re

DOWNSTREAM_ITEMS = {"FederationRevokePayload"}
PRODUCT_NEUTRAL_CUTOVER_REF = "docs/spec/daemon-sdk-requirements-v1.md#product-provider-surfaces"


def is_fallback_signer_item(item: str) -> bool:
    lowered = item.lower()
    tokens = semantic_tokens(item)
    compact = re.sub(r"[^a-z0-9]", "", lowered)
    explicit_names = {
        "defaultauthforsubject",
        "generatedsubjectauth",
        "generateprivateagentauth",
        "generateprivatehubauth",
        "generatesubjectauth",
        "generatedefaultauth",
        "defaultauthenticator",
        "processlocalsigner",
        "localgeneratedsigner",
        "generatedsigner",
        "privatekeyauth",
        "privatekeyauthenticator",
    }
    return (
        compact in explicit_names
        or lowered.endswith(".default_auth_for_subject")
        or lowered.endswith(".generate_private_agent_auth")
        or lowered.endswith(".generate_private_hub_auth")
        or lowered.endswith(".generate_subject_auth")
        or {"default", "auth", "subject"}.issubset(tokens)
        or {"generated", "subject", "auth"}.issubset(tokens)
        or {"generate", "private", "agent", "auth"}.issubset(tokens)
        or {"generate", "private", "hub", "auth"}.issubset(tokens)
        or {"generate", "subject", "auth"}.issubset(tokens)
        or {"process", "local", "signer"}.issubset(tokens)
        or {"private", "key", "auth"}.issubset(tokens)
    )


def public_root(item: str) -> str:
    return item.split("#", 1)[0].split(".", 1)[0]


def semantic_tokens(item: str) -> set[str]:
    text = item.split("#", 1)[0]
    tokens: set[str] = set()
    for part in re.split(r"[^A-Za-z0-9]+", text):
        if not part:
            continue
        for token in re.findall(r"[A-Z]+(?=[A-Z][a-z]|[0-9]|$)|[A-Z]?[a-z]+|[0-9]+", part):
            tokens.add(token.lower())
    return tokens


def is_ura_grammar_item(item: str) -> bool:
    root = public_root(item)
    lowered = item.lower()
    return (
        "ura" in root.lower()
        or "_ura" in lowered
        or "URA" in item
        or item in {
            "CanonicalURABuildRequest.DeviceID",
            "ParsedURA.DeviceID",
        }
    )


def canonical_quarantine_reason(item: str) -> str | None:
    root = public_root(item)
    tokens = semantic_tokens(item)
    lowered = item.lower()
    if is_fallback_signer_item(item):
        return "Process-local signer fallback is prohibited; canonical SDK signing uses an explicit signer handle or daemon KeyService authority."
    if root in {
        "canonical_invocation_bytes",
        "sign_invocation",
        "verify_invocation_signature",
        "verify_phase",
        "verify_signature",
        "run_admission",
    } or lowered.endswith((
        ".canonical_invocation_bytes",
        ".sign_invocation",
        ".verify_invocation_signature",
        ".verify_phase",
        ".verify_signature",
        ".run_admission",
    )):
        return "Plain canonical/admission helpers are transitional defects; canonical runtime entry is descriptor-bound proof."
    if public_root(item) in DOWNSTREAM_ITEMS:
        return "EasyNet federation payload is a downstream provider carrier, not a canonical SDK capability."
    if root in {
        "ControlDiscovery",
        "ControlDiscoveryReader",
        "ControlDiscoveryReaderFunc",
        "ControlDiscoveryRuntimeConnector",
        "ControlFrame",
        "ControlIpcClient",
        "FileControlDiscoveryReader",
        "NewControlDiscoveryFromJSON",
        "NewControlDiscoveryRuntimeConnector",
        "ResolveControlDiscoveryPath",
    }:
        return "Control discovery and raw control IPC are EasyNet provider boot/status surfaces, not canonical runtime SDK concepts."
    if root in {
        "RuntimeDeviceRevokeRequest",
        "RuntimeDeviceRevokeResult",
    }:
        return "Device revoke is EasyNet provider administration over federation.revoke, not product-neutral runtime administration."
    if root in {
        "ErrRuntimeIdentityNotFound",
        "ErrRuntimeIdentityUnavailable",
    }:
        return "Runtime identity error aliases are source-compatible names for daemon key-service errors, not independent canonical capability evidence."
    if root in {
        "DirectRuntimeConnector",
        "DirectRuntimeConnectorOptions",
        "DirectRuntimeOptions",
        "DirectRuntimeTransport",
        "NewDirectRuntimeConnector",
        "NewDirectRuntimeConnectorWithOptions",
        "OpenDirectRuntimeTransport",
    }:
        return "Direct runtime exports are EasyNet daemon provider/source-compatibility surface, not canonical SDK capability evidence."
    if root == "RuntimeHostRole":
        return "Current runtime host role values encode EasyNet device/hub topology and are provider/source-compatibility surface."
    if re.search(r"(?i)easynet|easyremote", item):
        return "Product-branded public compatibility surface; the canonical SDK runtime model must stay product-neutral."
    if re.search(r"(?i)remote_?desktop|voice(call|event|network|\b)", item):
        return "Product media/control carrier belongs to a downstream provider surface, not the canonical runtime capability model."
    if re.search(r"(?i)remote_?control|deploy|discover_nodes|resolve_tenant|abilitypackagedescriptor", item):
        return "Product deployment/control carrier belongs to a downstream provider surface, not the canonical runtime capability model."
    if "Daemon" in item or "daemon" in tokens:
        return "Daemon-bound provider or lifecycle surface; canonical SDK runtime concepts must not encode product daemon ownership."
    if "cabi" in tokens or "CABI" in item:
        return "C ABI transport/provider compatibility surface; canonical SDK runtime concepts must not encode provider binding names."
    retired_address_pattern = (
        r"(^|[^A-Za-z])" + "U" + r"RI([^A-Za-z]|$)"
        r"|\b" + "U" + r"ri\b"
        r"|(^|[^A-Za-z])" + "u" + r"ri([^A-Za-z]|$)"
    )
    if re.search(retired_address_pattern, item):
        return "Retired address-token naming is not part of the SDK architecture; canonical addressing is URA-only."
    if re.search(r"HubEndpoint|hub_endpoint", item):
        return "Hub endpoint fields expose a product directory/deployment model instead of a generic runtime concept."
    if {"device", "id"}.issubset(tokens) and not is_ura_grammar_item(item):
        return "Device identifiers outside URA grammar expose a product deployment model instead of generic runtime addressing."
    if {"device", "hub"} & tokens and not is_ura_grammar_item(item):
        return "Non-URA device/hub naming exposes product lifecycle or directory semantics inside the canonical SDK model."
    return None
