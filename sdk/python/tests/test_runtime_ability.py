from __future__ import annotations

import json

import pytest

import easynet_sdk.runtime_ability as runtime_ability_module
from easynet_sdk.axon_addressing import AddressingClient, AxonAddressingTransport
from easynet_sdk.errors import SDKError
from easynet_sdk.runtime import RuntimeClient
from easynet_sdk.runtime_ability import RuntimeAbilityClient, RuntimeCallContext


class RuntimeTransportFake:
    def __init__(self) -> None:
        self.seen: dict[str, object] = {}
        self.descriptor_requests: list[dict[str, object]] = []
        self.output_json: dict[str, object] = {"answer_kind": "positive"}

    def resolve_descriptor_ref(self, request_json: bytes) -> bytes:
        request = json.loads(request_json)
        self.descriptor_requests.append(request)
        action = "stream" if request["call_mode"] == "stream" else "read"
        return json.dumps(
            {
                "descriptor_ref": (
                    "easynet:///r/example/ability/hub.namespace.resolve@1.0.0#"
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    f"!{action}"
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


def _client() -> tuple[RuntimeAbilityClient, RuntimeTransportFake]:
    transport = RuntimeTransportFake()
    return (
        RuntimeAbilityClient(
            RuntimeClient(transport),  # type: ignore[arg-type]
            AddressingClient(AxonAddressingTransport()),
        ),
        transport,
    )


def _call() -> RuntimeCallContext:
    return RuntimeCallContext(
        caller_ura="easynet:///r/example/agent/alice.client",
        callee_ura="easynet:///r/example/hub",
        subject_ura="easynet:///r/example/user/alice",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        metadata={"request_id": "call-1"},
    )


def test_runtime_ability_builds_complete_canonical_draft() -> None:
    client, transport = _client()
    draft = client.build(_call(), "namespace.resolve", {"name": "alice"})
    assert draft.descriptor_ref == (
        "easynet:///r/example/ability/hub.namespace.resolve@1.0.0#"
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read"
    )
    assert transport.descriptor_requests == [
        {
            "ability": "namespace.resolve",
            "call_mode": "rpc",
            "callee_ura": _call().callee_ura,
            "caller_ura": _call().caller_ura,
            "subject_ura": _call().subject_ura,
        }
    ]
    assert draft.subject_ura != _call().subject_ura
    assert draft.metadata == {"request_id": "call-1"}


def test_runtime_ability_invokes_object_result() -> None:
    client, transport = _client()
    assert client.invoke(_call(), "namespace.resolve", {"name": "alice"}) == {
        "answer_kind": "positive"
    }
    assert transport.seen["descriptor_ref"] == (
        "easynet:///r/example/ability/hub.namespace.resolve@1.0.0#"
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read"
    )


def test_runtime_ability_stream_resolves_stream_descriptor() -> None:
    client, transport = _client()
    draft = client._build(_call(), "namespace.resolve", {}, call_mode="stream")
    assert draft.descriptor_ref.endswith("!stream")
    assert transport.descriptor_requests[-1]["call_mode"] == "stream"


def test_runtime_ability_requires_runtime_descriptor_resolver() -> None:
    class InvocationOnlyTransport:
        def invoke(self, draft_json: bytes) -> bytes:
            raise AssertionError(f"unexpected invocation: {draft_json!r}")

    client = RuntimeAbilityClient(
        RuntimeClient(InvocationOnlyTransport()),  # type: ignore[arg-type]
        AddressingClient(AxonAddressingTransport()),
    )
    with pytest.raises(SDKError, match="descriptor resolution"):
        client.build(_call(), "namespace.resolve", {})


def test_runtime_ability_rejects_incomplete_context() -> None:
    client, _ = _client()
    with pytest.raises(SDKError, match="causal_context is required"):
        client.build(
            RuntimeCallContext(
                caller_ura=_call().caller_ura,
                callee_ura=_call().callee_ura,
                subject_ura=_call().subject_ura,
                nonce_base64=_call().nonce_base64,
                causal_context=None,  # type: ignore[arg-type]
            ),
            "namespace.resolve",
            {},
        )


def test_runtime_ability_exports_only_canonical_contract() -> None:
    assert runtime_ability_module.__all__ == [
        "RuntimeAbilityClient",
        "RuntimeCallContext",
    ]
