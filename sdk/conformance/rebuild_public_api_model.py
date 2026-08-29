#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

from sdk_public_surface_policy import (
    non_canonical_public_reason,
)
from sdk_concepts import (
    STATUS_CANONICAL_NAMES,
    STATUSES,
    canonical_lifecycle_reference,
)

ROOT = Path(__file__).resolve().parents[2]
MODEL = ROOT / "sdk/conformance/canonical-public-api.json"
MATRIX = ROOT / "sdk/conformance/sdk-parity-matrix.json"
EXECUTION_MANIFEST = ROOT / "sdk/conformance/runner/execution-manifest.json"
LANGUAGES = ["rust", "c_abi", "go", "python", "node", "java", "swift"]
RETIRED_MODEL_FIELDS = {
    "lifecycle_actions": "duplicate_lifecycle_contract",
    "lifecycle_transition_contract": "duplicate_lifecycle_contract",
    "legacy_quarantine": "legacy_quarantine_retired",
}

# Ordered semantic rules are deliberately narrow. An item matching no rule is an
# inventory failure, not an implicit public/canonical assumption.
SEMANTIC_RULES: list[tuple[str, str]] = [
    (
        r"(abilitydispatch|abilitypackage|deployability|deploypackage|deploytonode|exportability|uninstallability|buildabilitypackage)",
        "ability_invocation_facade",
    ),
    (
        r"(mcptool|tooladapter|toolhandler|toolspec|progresssink|reportoutcome|^mcp$)",
        "unary_invoke",
    ),
    (
        r"(phasereceipt|receiptphase|newreceipt|deploytrace|phasestatus|^phase$|^receipt$)",
        "terminal_receipt_facts",
    ),
    (r"(capturemode|mediacapture)", "stream"),
    (r"(^modespec$)", "stream"),
    (r"(canonicalserialise)", "terminal_receipt_facts"),
    (r"(parsedescriptorfromargs)", "ability_descriptor_projection"),
    (r"(normalizeid)", "canonical_addressing"),
    (r"(normalizetags|parsebool|strfromvalue)", "typed_errors"),
    (r"(randomhex|unixmillis|unixseconds)", "complete_invocation_draft"),
    (r"(runfromargs|runfromenv)", "runtime_lifecycle"),
    (r"(^ability$)", "ability_invocation_facade"),
    (r"(^presets$)", "runtime_connection"),
    (r"(^utils$)", "typed_errors"),
    (
        r"(abilitydescriptor|descriptorref|descriptorprojection|abilityschema)",
        "ability_descriptor_projection",
    ),
    (
        r"(descriptorprovider|descriptorresolution|descriptor(resolved|notfound|owneroffline|modeunsupported|stale|unavailable))",
        "ability_descriptor_projection",
    ),
    (
        r"(accesscontrol|admission|grantactive|grantrevoked|noncereplay)",
        "access_control",
    ),
    (
        r"(delegation|sessionauthority|authority|proofbinding|hostedattestation|hostattestation|subjectauth|privateagentauth|privatehubauth)",
        "authority_metadata",
    ),
    (r"(authorityartifact|authorizationprovider)", "authority_metadata"),
    (
        r"(canonicaladdress|abilityaddress|abilityowner|addressing|abilityura|agentura|serviceura|entityura|receiptura|urakind|\bura\b|urabuilder|easynetaxonura|axonsdkura)",
        "canonical_addressing",
    ),
    (r"(bidi|duplex)", "bidi"),
    (
        r"(stream|leasedpayload|contentchunk|contentsegment|contentprovider|contentsession|segmenter)",
        "stream",
    ),
    (
        r"(receiptchain|verifyreceipt|verifiedreceipt|unverifiedreceipt|chaincheck|receiptverifier|finalizationcheckpoint|computereceipthash|verifyphase|verifyhostattestation)",
        "receipt_verification",
    ),
    (
        r"(sessionhistory|runtimecallcontext|receiptreadcallcontext|receiptfilter|receipthistory|receiptlist|receiptget|receiptquery|receiptanchor|receiptref|tracegraph|traceedge|ledgerrecord|invocationindex|persistentlog|causallink)",
        "receipt_history",
    ),
    (
        r"(terminalreceipt|runtimereceipt|boundreceipt|receiptfacts|receiptbody|receipttype|receiptjson|receiptproof|receiptprovider|hostedagentreceipt|canonicalreceipt|registeredimplementation|resolveddescriptor|receiptsigningauthority)",
        "terminal_receipt_facts",
    ),
    (
        r"(prepared|invocationprepare|signingmaterial|signedinvocation|signatureprovider|signerhandle|\bsigner\b)",
        "prepare_sign_submit",
    ),
    (
        r"(authorizedruntime|authorizedinvocation|signedinvocationstate|submittedinvocation|runtimesessionstate|runtimeclientsessionruntimeprovider|session.*operations)",
        "prepare_sign_submit",
    ),
    (
        r"(invocationdraft|invocationtuple|invocationbuilder|canonicalenvelope|wireenvelope|derivationpolicy|causal|agentref|agentidentity|subjectref|subjectidentity|entityref|invocationenvelope|invocationjson|canonicaljson|descriptorbound|freshnonce|privateagentsubject|privatehubsubject|validateenvelope|tojcs)",
        "complete_invocation_draft",
    ),
    (
        r"(abilityref|actingprincipalref|calleridentityref|clockidempotencysource|invocationintent|principalref|runtimetargetref)",
        "complete_invocation_draft",
    ),
    (r"(managedsign|keyservice)", "managed_signing"),
    (
        r"(runtimerecovery|invocationrecovery|restartrecover|recoverruntime|runtimerecover|recoveryphase|recoverysupervisor|recoverycontinuation|recoveryfn)",
        "runtime_lifecycle",
    ),
    (r"(principal.*recovery|recoverypolicy)", "principal_recovery"),
    (r"(principal.*enroll|enrollment)", "principal_enrollment"),
    (
        r"(publickeybinding|principal.*key|bindprincipal|rotateprincipal|revokeprincipalkey)",
        "principal_public_key_bindings",
    ),
    (r"(authorizationgrant|issuegrant|revokegrant)", "principal_authorization_grants"),
    (r"(principal)", "principal_lifecycle"),
    (
        r"(directory|discoverrequest|discoverresponse|resolveagent|resolvekey|resolver|federat|devicejoin|listuserdevices)",
        "directory_resolution",
    ),
    (r"(runtimeevent|abilitychangeevent|directoryevent)", "runtime_events"),
    (
        r"(runtimeidentity|runtimecredential|selfidentity|identityjson)",
        "runtime_identity",
    ),
    (r"(identityprovider)", "runtime_identity"),
    (r"(sdkenvironment|processroot|environment)", "runtime_environment"),
    (r"(runtimehealth|healthclient|healthtransport|diagnostic)", "runtime_health"),
    (r"(runtimeadmin|administration)", "runtime_administration"),
    (r"(controlipc|controlframe|controljson|controlendpoint)", "runtime_connection"),
    (r"(nativeruntime|cabiruntime|dendritebridge|nativebridge)", "native_runtime"),
    (
        r"(feature|abiversion|sdkversion|requireabi|version$|versioninfo)",
        "abi_version_discovery",
    ),
    (r"(runtimeconnection|reconnect|connection|connectoptions)", "runtime_connection"),
    (
        r"(runtimehost|serverconfig|serverhandle|startserver|attach|detach|runtimehandle|runtimemode|lifecycle|daemon|processhandle|supervisor|resource(limit|exhausted)|shutdown|easynetinit|stringfree|easynethandle|reaporphans|nowms)",
        "runtime_lifecycle",
    ),
    (
        r"(abilitycall|abilitytarget|childcontext|invocationobjectadapter|wireprojector|makeability)",
        "ability_invocation_facade",
    ),
    (
        r"(invocationhandle|invocationresult|invocationcancel|invocationterminalstate|finalizedinvocation|invocationsnapshot|invocationlimits|invocationusage|runtimeclient|invoke|callmode|localruntime|abilityregistry|abilitycontext|invocationcore|invocationstate|invocationcontrol|invocationevent|invocation.*tool|messageinbox|messageack|inboundmessage|inboxfull|toolschema|emitextras|terminalstates|valideventtypes|newinvocationid|handle$|messaging|easynetaxoninvocation|axonsdkinvocation|^invocation$)",
        "unary_invoke",
    ),
    (
        r"(authority(binding|evidence|relation|orbootstrap)|bootstrap(binding|json)|delegationevidence|sessionevidence|authorityorbootstrapjson|canonical(authority|bootstrap|delegation|session).*bytes|.*proofhash)",
        "receipt_verification",
    ),
    (
        r"(sdkerror|axonerror|errorcode|errorclass|retryhint|failure|errorstage|err[a-z]|mapprotocode)",
        "typed_errors",
    ),
    (
        r"(runtimeability|abilityoptions|abilityregistration|abilityframe|abilityfn|abilityraw|abilitysigned|publishcapability|childinvocation|abilityproof)",
        "ability_invocation_facade",
    ),
    (
        r"(backpressure|credit|tts|phrase|drain|producer|resumepolicy|loopback)",
        "stream",
    ),
    (r"(jsonvalue|jsonreader|jsonwriter|serialization)", "complete_invocation_draft"),
    (r"(default|max|zero|reason|securityclass|result$)", "typed_errors"),
    (r"(sign|keyresolver|pubkey|sha256)", "prepare_sign_submit"),
    (r"(ledger|audit|persistence|checkpoint)", "receipt_history"),
    (r"(axiom|bundle)", "complete_invocation_draft"),
    (r"(asynciteration|receiveoptions)", "stream"),
    (r"(profile)", "typed_errors"),
    (r"(^easynetaxon$|^axonsdk$)", "runtime_connection"),
    (r"(runtime|server)", "native_runtime"),
]


def normalize(value: str) -> str:
    value = value.split("#", 1)[0]
    root = value.split(".", 1)[0]
    if root[:1].isupper():
        value = root
    return re.sub(r"[^a-z0-9]", "", value.lower())


def support_shape_group(item: str, role: str) -> str:
    base, _, member = item.split("#", 1)[0].partition(".")
    key = re.sub(r"[^a-z0-9]", "", base.lower())
    operation = re.sub(r"[^a-z0-9]", "", member.lower())
    aliases = {
        "defaultreceipthistorylimit": "defaultreceiptpagelimit",
        "maxreceipthistorylimit": "maxreceiptpagelimit",
        "defaultruntimeeventpagelimit": "defaultruntimeeventpagelimit",
        "maxruntimeeventpagelimit": "maxruntimeeventpagelimit",
    }
    for source, target in aliases.items():
        key = key.replace(source, target)
    operation = {
        "marshaljson": "serializejson",
        "tojson": "serializejson",
        "todict": "serializejson",
        "fromjson": "deserializejson",
    }.get(operation, operation)
    if operation:
        key = f"{key}.{operation}"
    return f"{role}:{key}"


def support_parent(item: str, classified: str) -> str:
    key = re.sub(r"[^a-z0-9]", "", item.lower())
    for token, capability in (
        ("receipt", "receipt_history"),
        ("directory", "directory_resolution"),
        ("runtimeevent", "runtime_events"),
        ("bidi", "bidi"),
        ("stream", "stream"),
        ("managedsigning", "managed_signing"),
        ("controlframe", "runtime_connection"),
        ("runtimeidentity", "runtime_identity"),
    ):
        if token in key:
            return capability
    return classified


def reject_retired_model_fields(model: dict[str, Any]) -> None:
    for field, reason in RETIRED_MODEL_FIELDS.items():
        if field in model:
            raise ValueError(reason)


def owner_maps(
    model: dict[str, Any],
) -> tuple[dict[tuple[str, str], str], dict[str, set[str]]]:
    exact: dict[tuple[str, str], str] = {}
    aliases: dict[str, set[str]] = {}
    for capability, projection in model["capability_inventory"].items():
        aliases.setdefault(capability, set())
        for language in ("go", "python"):
            for section in ("symbols", "members"):
                for item in projection[capability if False else language][section]:
                    exact[(language, item)] = capability
                    aliases[capability].add(normalize(item))
    return exact, aliases


def classify(
    language: str,
    item: str,
    exact: dict[tuple[str, str], str],
    aliases: dict[str, set[str]],
) -> str:
    if (language, item) in exact:
        return exact[(language, item)]
    key = normalize(item)
    matches = [capability for capability, names in aliases.items() if key in names]
    if len(matches) == 1:
        return matches[0]
    for pattern, capability in SEMANTIC_RULES:
        if re.search(pattern, key):
            return capability
    raise ValueError(f"unclassified public item: {language}:{item}")


def inventory(language: str, cache: Path) -> dict[str, Any]:
    path = cache / f"{language}-api.json"
    subprocess.run(
        [
            sys.executable,
            str(ROOT / "sdk/conformance/public_api_inventory.py"),
            language,
            "--output",
            str(path),
        ],
        cwd=ROOT,
        check=True,
    )
    value = json.loads(path.read_text(encoding="utf-8"))
    if value["symbols"] != sorted(set(value["symbols"])) or value["members"] != sorted(
        set(value["members"])
    ):
        raise ValueError(f"inventory is not canonical: {language}")
    return value


def package_manifest() -> dict[str, list[dict[str, str]]]:
    go_output = subprocess.run(
        ["go", "list", "-f", "{{.Dir}}", "./..."],
        cwd=ROOT / "sdk/go",
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    go_categories = {
        "sdk/go": "public_facade",
        "sdk/go/directorycore": "provider_neutral_core",
        "sdk/go/internal/axonpb": "generated_wire",
        "sdk/go/internal/runtimeevents": "provider_neutral_core",
        "sdk/go/provider/runtime": "provider_neutral_core",
        "sdk/go/provider/runtime/pluginexec": "provider_neutral_core",
        "sdk/go/runtimeevents": "provider_neutral_core",
    }
    go_paths = sorted(str(Path(path).resolve().relative_to(ROOT)) for path in go_output)
    unknown = sorted(set(go_paths) - set(go_categories))
    if unknown:
        raise ValueError("unclassified Go package roots: " + ",".join(unknown))

    python_base = ROOT / "sdk/python/easynet_sdk"
    python_paths = sorted(
        str(path.relative_to(ROOT))
        for path in python_base.rglob("*")
        if path.is_dir() and (path / "__init__.py").is_file()
    )
    python_paths.insert(0, "sdk/python/easynet_sdk")
    python_paths = sorted(set(python_paths))
    python_categories: dict[str, str] = {}
    for path in python_paths:
        if "/providers/easynet" in path:
            category = "distribution_facade"
        elif "/providers/runtime" in path:
            category = "provider_neutral_core"
        elif "/providers" in path:
            category = "provider_registry"
        elif "/core" in path:
            category = "provider_neutral_core"
        elif "/_axon_pb" in path:
            category = "generated_wire"
        elif path == "sdk/python/easynet_sdk":
            category = "distribution_facade"
        else:
            category = "public_facade"
        python_categories[path] = category
    return {
        "rust": [
            {"path": "../EasyNet-Axon/sdk/rust", "category": "canonical_axon_sdk"}
        ],
        "c_abi": [{"path": "include/easynet_cli.h", "category": "public_abi"}],
        "go": [{"path": path, "category": go_categories[path]} for path in go_paths],
        "python": [
            {"path": path, "category": python_categories[path]} for path in python_paths
        ],
        "node": [{"path": "sdk/node/index.d.ts", "category": "public_facade"}],
        "java": [
            {
                "path": "sdk/java/src/main/java/run/runtime/sdk",
                "category": "distribution_facade",
            }
        ],
        "swift": [
            {
                "path": "sdk/swift/Sources/RuntimeSDK",
                "category": "distribution_facade",
            }
        ],
    }


def refresh_provider_proof_implementations(model: dict[str, Any]) -> None:
    proofs = model.get("provider_proofs")
    implementations = model.get("provider_implementations")
    if not isinstance(proofs, dict) or not isinstance(implementations, list):
        raise ValueError("provider proofs and implementations are required")
    manifest = json.loads(EXECUTION_MANIFEST.read_text(encoding="utf-8"))
    manifest_bindings: dict[tuple[str, str], str] = {}
    for binding in manifest.get("bindings", []):
        if not isinstance(binding, dict):
            raise ValueError("invalid execution manifest binding")
        key = (binding.get("language"), binding.get("case_id"))
        selector = binding.get("selector")
        if (
            not all(isinstance(value, str) and value for value in (*key, selector))
            or key in manifest_bindings
        ):
            raise ValueError(f"invalid or duplicate execution manifest binding: {key}")
        manifest_bindings[key] = selector

    for capability_id, languages in proofs.items():
        if not isinstance(languages, dict):
            raise ValueError(f"invalid provider proof set: {capability_id}")
        for language, proof in languages.items():
            if not isinstance(proof, dict):
                raise ValueError(f"invalid provider proof: {capability_id}:{language}")
            matches = [
                implementation
                for implementation in implementations
                if implementation["language"] == language
                and capability_id in implementation["capability_ids"]
            ]
            if len(matches) != 1:
                raise ValueError(
                    "provider proof must resolve to one registered implementation: "
                    f"{capability_id}:{language}"
                )
            implementation = matches[0]
            proof["implementation"] = {
                "identity": implementation["identity"],
                "owner_path": implementation["owner_path"],
                "path": implementation["path"],
                "sha256": implementation["sha256"],
                "interface": implementation["interface"],
                "revision": implementation["axon_revision"],
            }
            for step in proof.get("step_evidence", []):
                if not isinstance(step, dict):
                    raise ValueError(
                        f"invalid provider proof step: {capability_id}:{language}"
                    )
                key = (language, step.get("case_id"))
                selector = manifest_bindings.get(key)
                if selector is None:
                    raise ValueError(
                        "provider proof step is not execution-bound: "
                        f"{capability_id}:{language}:{step.get('case_id')}"
                    )
                step["selector"] = selector


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--cache", type=Path, default=ROOT / "target/sdk-public-api-inventory"
    )
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    args.cache.mkdir(parents=True, exist_ok=True)
    model = json.loads(MODEL.read_text(encoding="utf-8"))
    reject_retired_model_fields(model)
    control_ipc = model.get("capabilities", {}).pop("control_ipc", None)
    if control_ipc is not None:
        runtime_connection = model["capabilities"]["runtime_connection"]
        runtime_cases = runtime_connection["case_ids"]
        for case_id in control_ipc.get("case_ids", []):
            if case_id not in runtime_cases:
                runtime_cases.append(case_id)
        runtime_cases.sort()
        model.get("capability_inventory", {}).pop("control_ipc", None)
    exact, aliases = owner_maps(model)
    inventories = {language: inventory(language, args.cache) for language in LANGUAGES}
    axon_revision = inventories["rust"].get("source_revision")
    if not isinstance(axon_revision, str) or not axon_revision:
        raise ValueError("rust inventory source revision is required")
    for language in ("rust", "python"):
        if inventories[language].get("source_revision") != axon_revision:
            raise ValueError(
                f"{language} inventory source revision mismatch: "
                f"expected={axon_revision}:actual={inventories[language].get('source_revision')}"
            )

    model["schema_version"] = 6
    model["status_order"] = STATUSES
    model["status_canonical_names"] = STATUS_CANONICAL_NAMES
    model["canonical_lifecycle_contract"] = canonical_lifecycle_reference()
    model["inventory_parsers"] = {
        language: inventories[language]["parser"] for language in LANGUAGES
    }
    model["inventory_source_revisions"] = {
        language: inventories[language].get("source_revision", "current_checkout")
        for language in LANGUAGES
    }
    model["languages"] = {}
    model["members"] = {}
    model["non_canonical"] = {
        "languages": {language: [] for language in LANGUAGES},
        "members": {language: [] for language in LANGUAGES},
    }
    model["shape_sha256"] = {}
    for language, found in inventories.items():
        model["languages"][language] = []
        model["members"][language] = []
        for section, source_items in (
            ("languages", found["symbols"]),
            ("members", found["members"]),
        ):
            for item in source_items:
                reason = non_canonical_public_reason(item)
                if reason is not None:
                    raise ValueError(
                        f"non_canonical_public_item:{language}:{section}:{item}:{reason}"
                    )
                model[section][language].append(item)
        model["shape_sha256"][language] = {
            item: hashlib.sha256(shape.encode()).hexdigest()
            for item, shape in found["shapes"].items()
        }

    for projection in model["capabilities"].values():
        if isinstance(projection.get("case_ids"), list):
            projection["case_ids"] = sorted(set(projection["case_ids"]))

    for capability in model["capabilities"]:
        model["capability_inventory"][capability] = {
            language: {"symbols": [], "members": []} for language in LANGUAGES
        }
    support_items: list[dict[str, str]] = []
    support_lookup: dict[tuple[str, str], str] = {}
    old_support = model.pop("non_capability_classification", None)
    if old_support is not None:
        for role, definition in old_support.items():
            for language in ("go", "python"):
                for section in ("symbols", "members"):
                    for item in definition["projection"][language][section]:
                        support_lookup[(language, item)] = role
    else:
        for item in model.get("supporting_items", []):
            support_lookup[(item["language"], item["item"])] = item["role"]

    for language in LANGUAGES:
        for section in ("symbols", "members"):
            for item in model["languages" if section == "symbols" else "members"][
                language
            ]:
                capability = classify(language, item, exact, aliases)
                support_role = support_lookup.get((language, item))
                if support_role:
                    capability = support_parent(item, capability)
                    support_items.append(
                        {
                            "language": language,
                            "section": section,
                            "item": item,
                            "parent_capability": capability,
                            "role": support_role,
                            "shape_sha256": model["shape_sha256"][language][item],
                            "shape_group": support_shape_group(item, support_role),
                        }
                    )
                else:
                    model["capability_inventory"][capability][language][section].append(
                        item
                    )
    for projection in model["capability_inventory"].values():
        if isinstance(projection.get("case_ids"), list):
            projection["case_ids"] = sorted(set(projection["case_ids"]))
        for language in LANGUAGES:
            projection[language]["symbols"].sort()
            projection[language]["members"].sort()
    model["supporting_items"] = sorted(
        support_items,
        key=lambda item: (item["language"], item["section"], item["item"]),
    )
    grouped_support: dict[str, dict[str, Any]] = {}
    for item in model["supporting_items"]:
        group = grouped_support.setdefault(
            item["shape_group"],
            {
                "parent_capability": item["parent_capability"],
                "role": item["role"],
                "members": [],
            },
        )
        if (
            group["parent_capability"] != item["parent_capability"]
            or group["role"] != item["role"]
        ):
            raise ValueError(
                f"support shape group crosses semantic owners: {item['shape_group']}"
            )
        group["members"].append(
            {
                "language": item["language"],
                "section": item["section"],
                "item": item["item"],
                "shape_sha256": item["shape_sha256"],
            }
        )
    model["support_shape_groups"] = {
        name: {
            **group,
            "members": sorted(
                group["members"],
                key=lambda member: (
                    member["language"],
                    member["section"],
                    member["item"],
                ),
            ),
            "present_languages": sorted(
                {member["language"] for member in group["members"]}, key=LANGUAGES.index
            ),
            "missing_languages": [
                language
                for language in LANGUAGES
                if language not in {member["language"] for member in group["members"]}
            ],
        }
        for name, group in sorted(grouped_support.items())
    }
    model["canonical_packages"] = package_manifest()
    model["dependency_revisions"] = {"axon_sdk": axon_revision}
    revision = model["dependency_revisions"]["axon_sdk"]
    runtime_provider_capabilities = [
        "native_runtime",
        "runtime_environment",
        "runtime_connection",
        "runtime_lifecycle",
        "terminal_receipt_facts",
        "unary_invoke",
    ]
    stream_provider_capabilities = ["stream", "bidi"]
    ability_provider_capabilities = ["ability_invocation_facade"]
    provider_specs = [
        (
            "go",
            "direct_runtime_provider",
            "sdk/go/",
            "sdk/go/direct_runtime.go",
            "RuntimeConnector",
            "sdk/go/connection.go",
            runtime_provider_capabilities,
        ),
        (
            "python",
            "direct_runtime_provider",
            "sdk/python/easynet_sdk/providers/runtime/",
            "sdk/python/easynet_sdk/providers/runtime/direct.py",
            "RuntimeConnector",
            "sdk/python/easynet_sdk/connection.py",
            runtime_provider_capabilities,
        ),
        (
            "go",
            "cabi_runtime_provider",
            "sdk/go/",
            "sdk/go/cabi_runtime.go",
            "RuntimeTransport",
            "sdk/go/runtime.go",
            stream_provider_capabilities,
        ),
        (
            "python",
            "cabi_runtime_provider",
            "sdk/python/easynet_sdk/",
            "sdk/python/easynet_sdk/_cabi.py",
            "CABIRuntimeTransport",
            "sdk/python/easynet_sdk/_cabi.py",
            stream_provider_capabilities,
        ),
        (
            "go",
            "ability_invocation_facade",
            "sdk/go/",
            "sdk/go/runtime_ability.go",
            "RuntimeAbilityClient",
            "sdk/go/runtime_ability.go",
            ability_provider_capabilities,
        ),
        (
            "python",
            "ability_invocation_facade",
            "sdk/python/easynet_sdk/",
            "sdk/python/easynet_sdk/ability_invocation.py",
            "AbilityInvocationClient",
            "sdk/python/easynet_sdk/ability_invocation.py",
            ability_provider_capabilities,
        ),
    ]
    model["provider_implementations"] = []
    for (
        language,
        identity,
        owner_path,
        path,
        interface,
        interface_path,
        capabilities,
    ) in provider_specs:
        source = ROOT / path
        interface_source = ROOT / interface_path
        model["provider_implementations"].append(
            {
                "language": language,
                "identity": identity,
                "production_owner": "canonical runtime provider",
                "owner_path": owner_path,
                "path": path,
                "sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
                "interface": interface,
                "interface_path": interface_path,
                "interface_sha256": hashlib.sha256(
                    interface_source.read_bytes()
                ).hexdigest(),
                "capability_ids": capabilities,
                "axon_revision": revision,
            }
        )
    refresh_provider_proof_implementations(model)
    encoded = json.dumps(model, indent=2, sort_keys=False) + "\n"
    if args.write:
        MODEL.write_text(encoded, encoding="utf-8")
        import sdk_matrix

        MATRIX.write_text(
            json.dumps(sdk_matrix.generate(), indent=2) + "\n",
            encoding="utf-8",
        )
    else:
        print(encoded, end="")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (
        OSError,
        ValueError,
        subprocess.CalledProcessError,
        json.JSONDecodeError,
    ) as error:
        print(f"rebuild_public_api_model: {error}", file=sys.stderr)
        raise SystemExit(1)
