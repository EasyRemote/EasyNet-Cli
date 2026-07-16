from __future__ import annotations

import json

import pytest

from easynet_sdk.ability_descriptor import (
    AbilityDescriptorGetRequest,
    AbilityDescriptorListRequest,
    RuntimeAbilityDescriptorProvider,
    project_ability_descriptor,
)
from easynet_sdk.axon_addressing import AddressingClient, AxonAddressingTransport
from easynet_sdk.errors import ErrorCode, SDKError, is_code
from easynet_sdk.runtime import RuntimeClient
from easynet_sdk.runtime_ability import RuntimeAbilityClient, RuntimeCallContext


class RuntimeTransportFake:
    def __init__(self) -> None:
        self.seen: dict[str, object] = {}
        self.output_json: dict[str, object] = {"abilities": []}

    def resolve_descriptor_ref(self, request_json: bytes) -> bytes:
        request = json.loads(request_json)
        return json.dumps(
            {
                "descriptor_ref": (
                    "easynet:///r/example/ability/hub."
                    f"{request['ability']}@1.0.0"
                )
            }
        ).encode()

    def invoke(self, draft_json: bytes) -> bytes:
        self.seen = json.loads(draft_json)
        return json.dumps(
            {
                "ok": True,
                "tuple": self.seen,
                "invocation_id": "inv-1",
                "terminal_state": "Completed",
                "output_content_type": "application/json",
                "output_json": self.output_json,
                "elapsed_ms": 1,
                "error": None,
            }
        ).encode()


def _provider() -> tuple[RuntimeAbilityDescriptorProvider, RuntimeTransportFake]:
    transport = RuntimeTransportFake()
    ability = RuntimeAbilityClient(
        RuntimeClient(transport),  # type: ignore[arg-type]
        AddressingClient(AxonAddressingTransport()),
    )
    return RuntimeAbilityDescriptorProvider(ability), transport


def _call() -> RuntimeCallContext:
    return RuntimeCallContext(
        caller_ura="easynet:///r/example/agent/alice.client",
        callee_ura="easynet:///r/example/hub",
        subject_ura="easynet:///r/example/user/alice",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        metadata={"request_id": "call-1"},
    )


def test_project_ability_descriptor_merges_nested_descriptor() -> None:
    projection = project_ability_descriptor(
        {
            "descriptor": {
                "name": "skill.list",
                "owner_ura": "easynet:///r/localhost/device/node-a",
                "ability_ura": "easynet:///r/localhost/ability/device.node-a.skill.list",
                "metadata": {"tool_name": "skill.list"},
            },
            "name": "agent.list",
        }
    )

    assert projection.name == "agent.list"
    assert projection.owner_ura == "easynet:///r/localhost/device/node-a"
    assert (
        projection.ability_ura
        == "easynet:///r/localhost/ability/device.node-a.skill.list"
    )
    assert projection.metadata["tool_name"] == "skill.list"


def test_runtime_ability_descriptor_provider_lists_daemon_descriptors() -> None:
    provider, transport = _provider()
    transport.output_json = {
        "abilities": [
            {
                "name": "namespace.resolve",
                "ability_ura": "easynet:///r/example/ability/hub.namespace.resolve",
                "descriptor_ref": "easynet:///r/example/ability/hub.namespace.resolve@1.0.0",
                "owner_ura": "easynet:///r/example/hub",
                "descriptor_version": "1.0.0",
                "schema_hash": "sha256:abc",
                "descriptor_hash": "sha256:def",
                "call_mode": "rpc",
                "class": "runtime",
                "receipt_semantics": {"kind": "terminal"},
                "visibility": "public",
                "description": "Resolve names",
                "source": "kernel:built-in",
                "hints": {"read_only": True, "idempotent": True},
                "schema_summary": {"input": {"type": "object"}},
                "metadata": {"stable": "true"},
            }
        ]
    }

    page = provider.list(
        AbilityDescriptorListRequest(
            call=_call(),
            scope="realm",
            owner_ura="easynet:///r/example/hub",
        )
    )

    assert transport.seen["descriptor_ref"] == (
        "easynet:///r/example/ability/hub.meta.list_abilities@1.0.0"
    )
    assert transport.seen["args"] == {
        "scope": "realm",
        "agent_ura": "easynet:///r/example/hub",
    }
    assert len(page.descriptors) == 1
    descriptor = page.descriptors[0]
    assert descriptor.ability_ura == "easynet:///r/example/ability/hub.namespace.resolve"
    assert (
        descriptor.descriptor_ref
        == "easynet:///r/example/ability/hub.namespace.resolve@1.0.0"
    )
    assert descriptor.version == "1.0.0"
    assert descriptor.class_ == "runtime"
    assert descriptor.schema_hash == "sha256:abc"
    assert descriptor.call_mode == "rpc"
    assert descriptor.hints.read_only
    assert descriptor.schema_summary["input"]
    assert descriptor.input_schema["type"] == "object"
    assert descriptor.metadata["stable"] == "true"


def test_runtime_ability_descriptor_provider_get_rejects_ambiguous_descriptors() -> None:
    provider, transport = _provider()
    transport.output_json = {
        "abilities": [
            {
                "name": "observe.health",
                "ability_ura": "easynet:///r/example/ability/hub.observe.health",
                "owner_ura": "easynet:///r/example/hub",
                "version": "1.0.0",
                "call_mode": "rpc",
            },
            {
                "name": "observe.health",
                "ability_ura": "easynet:///r/example/ability/hub.observe.health",
                "owner_ura": "easynet:///r/example/hub",
                "version": "2.0.0",
                "call_mode": "rpc",
            },
        ]
    }

    with pytest.raises(SDKError) as caught:
        provider.get(
            AbilityDescriptorGetRequest(
                call=_call(),
                ability_ura="easynet:///r/example/ability/hub.observe.health",
            )
        )
    assert is_code(caught.value, ErrorCode.INVALID_ARGUMENT)

    descriptor = provider.get(
        AbilityDescriptorGetRequest(
            call=_call(),
            ability_ura="easynet:///r/example/ability/hub.observe.health",
            descriptor_version="2.0.0",
        )
    )
    assert descriptor.version == "2.0.0"
