from __future__ import annotations

import json

import pytest

import easynet_sdk.runtime_ability as runtime_ability_module
from easynet_sdk.axon_addressing import AddressingClient, AxonAddressingTransport
from easynet_sdk.errors import SDKError
from easynet_sdk.bidi import BidiStreamDescriptor
from easynet_sdk.runtime import RuntimeClient, RuntimeRecoveryRequest
from easynet_sdk.runtime_ability import RuntimeAbilityClient, RuntimeCallContext


class RuntimeTransportFake:
    def __init__(self) -> None:
        self.seen: dict[str, object] = {}
        self.seen_stream: dict[str, object] = {}
        self.seen_bidi: dict[str, object] = {}
        self.seen_streams: list[dict[str, object]] = []
        self.seen_signed: dict[str, object] = {}
        self.seen_recovery_request: dict[str, object] = {}
        self.descriptor_requests: list[dict[str, object]] = []
        self.output_json: dict[str, object] = {"answer_kind": "positive"}
        self.seen_await_id = 0
        self.seen_cancel_reason = ""
        self.seen_free_id = 0
        self.closed = False

    def resolve_descriptor_ref(self, request_json: bytes) -> bytes:
        request = json.loads(request_json)
        self.descriptor_requests.append(request)
        action = {
            "bidi": "bidi",
            "rpc": "read",
            "stream": "stream",
        }[request["call_mode"]]
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

    def open_stream(self, draft_json: bytes):
        self.seen_stream = json.loads(draft_json)
        from test_stream import MemoryStreamTransport

        return (
            MemoryStreamTransport(
                [
                    b'{"sequence":1,"kind":"terminal","state":"Completed","terminal":true}'
                ]
            ),
            b'{"stream_id":"stream-1","state":"Opening","max_buffered_events":4}',
        )

    def open_bidi(self, draft_json: bytes, streams_json: bytes):
        self.seen_bidi = json.loads(draft_json)
        self.seen_streams = json.loads(streams_json)
        from test_bidi import MemoryBidiTransport

        return (
            MemoryBidiTransport(),
            b'{"session_id":"bidi-1","state":"Open","max_buffered_frames":4}',
        )

    def await_handle(self, control) -> bytes:
        self.seen_await_id = control._adapter_handle_id()
        return json.dumps(
            {
                "ok": True,
                "tuple": self.seen or _complete_draft_json(),
                "invocation_id": "inv-1",
                "terminal_state": "Completed",
                "output_content_type": "application/json",
                "output_json": {},
                "elapsed_ms": 1,
                "error": None,
            }
        ).encode()

    def submit_signed(self, signed_json: bytes) -> bytes:
        self.seen_signed = json.loads(signed_json)
        return b'{"handle_id":7,"state":"Submitted","terminal":false}'

    def recover(self, request_json: bytes) -> bytes:
        self.seen_recovery_request = json.loads(request_json)
        return json.dumps(
            {
                "bounded_scan": True,
                "cleanup_complete": True,
                "events": [
                    {
                        "invocation_id": "inv-orphan",
                        "kind": "orphan_reaped",
                        "receipt_ura": "easynet:///r/example/resource/agent.alice/invocation/inv-orphan/receipt",
                        "sequence": 1,
                        "state": "cancelled",
                        "terminal": True,
                    }
                ],
                "reaped_orphans": 1,
                "recovered_invocations": 2,
                "recovery_id": "ability-recovery-1",
                "replayed_terminal_receipts": 1,
                "state": "runtime_started",
            }
        ).encode()

    def cancel_handle(self, control, reason: str) -> bytes:
        self.seen_cancel_reason = reason
        return (
            b'{"handle_id":7,"request_accepted":true,"deduplicated":false,'
            b'"cancelled":true,"state":"CancelRequested","terminal":false}'
        )

    def handle_events(self, control) -> bytes:
        return (
            b'{"handle_id":7,"state":"Submitted","terminal":false,'
            b'"events":[{"sequence":1,"kind":"submitted",'
            b'"state":"Submitted","terminal":false}],"result":null}'
        )

    def free_handle(self, control) -> None:
        self.seen_free_id = control._adapter_handle_id()

    def close(self) -> None:
        self.closed = True


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
    stream = client.open_stream(_call(), "namespace.resolve", {})
    assert stream.stream_id == "stream-1"
    assert transport.seen_stream["descriptor_ref"].endswith("!stream")
    assert transport.descriptor_requests[-1]["call_mode"] == "stream"


def test_runtime_ability_bidi_resolves_bidi_descriptor() -> None:
    client, transport = _client()
    session = client.open_bidi(
        _call(),
        "namespace.resolve",
        {},
        (BidiStreamDescriptor(stream_id=1, content_type="application/json"),),
    )
    assert session.session_id == "bidi-1"
    assert transport.seen_bidi["descriptor_ref"].endswith("!bidi")
    assert transport.seen_streams == [
        {"content_type": "application/json", "stream_id": 1}
    ]
    assert transport.descriptor_requests[-1]["call_mode"] == "bidi"


def test_runtime_ability_delegates_provider_handle_lifecycle() -> None:
    client, transport = _client()
    from test_runtime import signed_fixture

    handle = client.submit_signed(signed_fixture())

    result = client.await_result(handle)
    cancelled = client.cancel(handle, "client stop")
    refreshed = client.events(handle)
    client.close_handle(handle)

    assert result.ok
    assert transport.seen_signed["signature"]["signature_base64"]
    assert transport.seen_await_id == 7
    assert cancelled.request_accepted
    assert cancelled.cancelled
    assert not cancelled.terminal
    assert refreshed.state == "Submitted"
    assert transport.seen_cancel_reason == "client stop"
    assert transport.seen_free_id == 7


def test_runtime_ability_delegates_restart_recovery() -> None:
    client, transport = _client()

    report = client.recover(
        RuntimeRecoveryRequest(
            recovery_id="ability-recovery-1",
            deadline_unix_ms=1783100009999,
            max_invocations=32,
        )
    )

    assert transport.seen_recovery_request == {
        "deadline_unix_ms": 1783100009999,
        "max_invocations": 32,
        "recovery_id": "ability-recovery-1",
    }
    assert report.state == "runtime_started"
    assert report.bounded_scan
    assert report.cleanup_complete
    assert report.events[0].kind == "orphan_reaped"


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


def _complete_draft_json() -> dict[str, object]:
    return {
        "caller_ura": _call().caller_ura,
        "callee_ura": _call().callee_ura,
        "descriptor_ref": (
            "easynet:///r/example/ability/hub.namespace.resolve@1.0.0#"
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read"
        ),
        "subject_ura": "easynet:///r/example/resource/user.alice/invoke/namespace.resolve",
        "nonce_base64": _call().nonce_base64,
        "causal_context": {"form": "none"},
        "args": {},
        "content_type": "application/json",
    }
