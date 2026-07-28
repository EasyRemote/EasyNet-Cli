from __future__ import annotations

import base64
import hashlib
import json
from pathlib import Path
from typing import Any

import pytest

import easynet_sdk.receipt as receipt_module
from easynet_sdk._receipt_routes import _RECEIPT_ROUTE_MANIFEST_SHA256
from easynet_sdk._runtime_subjects import runtime_state_read_subject_ura
from easynet_sdk.authority import DelegationProof, SessionAuthority
from easynet_sdk.axon_addressing import AddressingClient, AxonAddressingTransport
from easynet_sdk.errors import ErrorCode, SDKError
from easynet_sdk.receipt import (
    DEFAULT_RECEIPT_PAGE_LIMIT,
    MAX_RECEIPT_PAGE_LIMIT,
    InvocationLedgerRecord,
    InvocationReceiptAnchor,
    ReceiptClient,
    ReceiptFilter,
    ReceiptGetRequest,
    ReceiptListRequest,
    ReceiptLookup,
    ReceiptReference,
    ReceiptTraceRequest,
    RuntimeReceiptProvider,
)
from easynet_sdk.runtime import RuntimeClient
from easynet_sdk.runtime_ability import RuntimeAbilityClient, RuntimeCallContext

from test_runtime_ability import RuntimeTransportFake, _call


LEDGER_URA = "easynet:///r/example/resource/device.node/billing/invocations"


def test_receipt_routes_are_generated_from_manifest() -> None:
    manifest = (
        Path(__file__).resolve().parents[2]
        .parent
        / "provider_routes"
        / "runtime-receipt-routes.v1.json"
    )
    digest = hashlib.sha256(manifest.read_bytes()).hexdigest()
    assert _RECEIPT_ROUTE_MANIFEST_SHA256 == digest


def _record(
    *, request_id: str = "request-1", trace_id: str = "trace-1"
) -> dict[str, Any]:
    return {
        "invocation_ura": (
            f"easynet:///r/example/resource/alice.invocations/{request_id}"
        ),
        "request_id": request_id,
        "trace_id": trace_id,
        "span_id": "span-1",
        "caller_ura": "easynet:///r/example/user/alice",
        "callee_ura": "easynet:///r/example/agent/alice.worker",
        "subject_ura": "easynet:///r/example/resource/alice.docs/report",
        "ability_ura": "easynet:///r/example/ability/alice.worker.docs.read",
        "ability_name": "docs.read",
        "state": "completed",
        "started_unix_ms": 1,
        "completed_unix_ms": 2,
        "elapsed_ms": 1,
        "args": {},
        "result": None,
        "error": None,
        "diagnostics": [],
        "causal_links": [],
        "receipt_chain": {},
        "visibility": {},
        "authority_form": "self",
        "usage": {},
    }


def _provider() -> tuple[RuntimeReceiptProvider, RuntimeTransportFake]:
    class ReceiptRuntimeTransport(RuntimeTransportFake):
        def resolve_descriptor_ref(self, request_json: bytes) -> bytes:
            request = json.loads(request_json)
            self.descriptor_requests.append(request)
            owner_prefix = _ability_owner_prefix(str(request["callee_ura"]))
            return json.dumps(
                {
                    "descriptor_ref": (
                        "easynet:///r/example/ability/"
                        f"{owner_prefix}."
                        f"{request['ability']}@1.0.0#"
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        "!read"
                    )
                }
            ).encode()

    transport = ReceiptRuntimeTransport()
    ability = RuntimeAbilityClient(
        RuntimeClient(transport),  # type: ignore[arg-type]
        AddressingClient(AxonAddressingTransport()),
    )
    return RuntimeReceiptProvider(ability), transport


def _ability_owner_prefix(callee_ura: str) -> str:
    if "/authority" in callee_ura:
        return "authority"
    if "/agent/" in callee_ura:
        return callee_ura.rsplit("/agent/", 1)[1].strip()
    if "/device/" in callee_ura:
        return f"device.{callee_ura.rsplit('/device/', 1)[1].strip()}"
    if "/user/" in callee_ura:
        return f"user.{callee_ura.rsplit('/user/', 1)[1].strip()}"
    raise AssertionError(f"unsupported test callee URA: {callee_ura}")


def _output(**values: object) -> dict[str, object]:
    return {
        "ledger_ura": LEDGER_URA,
        **values,
    }


def _history_call(
    *,
    caller_ura: str = "easynet:///r/example/agent/backend",
    callee_ura: str = "easynet:///r/example/authority",
    scope: str = "invocation.history.*",
    followup_ability: str = "invocation.history.list",
) -> RuntimeCallContext:
    subject_ura = runtime_state_read_subject_ura("example", "alice")
    payload = {
        "issuer_ura": caller_ura,
        "session_id": "session-1",
        "session_owner_user_id": "alice",
        "creator_principal_id": caller_ura,
        "callee_ura": callee_ura,
        "subject_ura": "easynet:///r/example/resource/user.alice/session/session-1",
        "audience": callee_ura,
        "scopes": [scope],
        "allowed_actions": ["read"],
        "allowed_followup_abilities": [followup_ability],
        "issued_at_ms": 1000,
        "expires_at_ms": 2000,
    }
    return RuntimeCallContext(
        caller_ura=caller_ura,
        callee_ura=callee_ura,
        subject_ura=subject_ura,
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        metadata={"request_id": "call-1"},
        authority=SessionAuthority.from_metadata(
            _authority_metadata_value(payload, b"session-signature")
        ),
    )


def _authority_metadata_value(payload: dict[str, object], signature: bytes) -> str:
    return base64.b64encode(
        json.dumps(
            {
                "payload": payload,
                "signature": base64.b64encode(signature).decode("ascii"),
            },
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")
    ).decode("ascii")


def test_receipt_reference_uses_canonical_axon_scalar_projection() -> None:
    reference = ReceiptReference(
        receipt_ura="  easynet:///r/example/resource/alice.invocations/request-1/receipt/1  ",
        receipt_hash=b"\xab" * 32,
    )
    assert reference.causal_context() == {
        "form": "scalar",
        "receipt_hash_hex": "ab" * 32,
        "receipt_ura": (
            "easynet:///r/example/resource/alice.invocations/request-1/receipt/1"
        ),
    }

    anchored = ReceiptReference.from_anchor(
        InvocationReceiptAnchor(
            receipt_ura=reference.receipt_ura,
            receipt_hash="ab" * 32,
            receipt_type="terminal",
            state="completed",
            timestamp_unix_ms=1,
        )
    )
    assert anchored == reference


@pytest.mark.parametrize("receipt_hash", [b"", b"a" * 31, b"a" * 33, "aa" * 32])
def test_receipt_reference_rejects_non_32_byte_hash(receipt_hash: object) -> None:
    with pytest.raises(SDKError) as caught:
        ReceiptReference(
            receipt_ura="easynet:///r/example/resource/alice.invocations/r/receipt/1",
            receipt_hash=receipt_hash,  # type: ignore[arg-type]
        )
    assert caught.value.code == ErrorCode.INVALID_ARGUMENT
    assert caught.value.stage == "receipt"


def test_receipt_reference_requires_daemon_or_axon_issued_ura() -> None:
    with pytest.raises(SDKError, match="receipt_ura is required"):
        ReceiptReference(receipt_ura=" ", receipt_hash=b"a" * 32)
    with pytest.raises(SDKError, match="must be a canonical URA"):
        ReceiptReference(
            receipt_ura="https://example.invalid/r", receipt_hash=b"a" * 32
        )


def test_receipt_reference_from_runtime_receipt_summary() -> None:
    reference = ReceiptReference.from_runtime_receipt(
        {
            "receipt_ura": (
                "easynet:///r/example/resource/alice.invocations/request-1/receipt/1"
            ),
            "self_hash_hex": "cd" * 32,
        }
    )
    assert reference.receipt_ura == (
        "easynet:///r/example/resource/alice.invocations/request-1/receipt/1"
    )
    assert reference.receipt_hash == b"\xcd" * 32

    with pytest.raises(SDKError, match="missing receipt_ura"):
        ReceiptReference.from_runtime_receipt({"self_hash_hex": "cd" * 32})
    with pytest.raises(SDKError, match="self_hash_hex must be hexadecimal"):
        ReceiptReference.from_runtime_receipt(
            {
                "receipt_ura": reference.receipt_ura,
                "self_hash_hex": "not-hex",
            }
        )


def test_runtime_receipt_list_projects_typed_query_and_axon_record() -> None:
    provider, transport = _provider()
    transport.output_json = _output(
        records=[_record()], next_cursor="receipt-history:v1:cursor-1"
    )
    call = _history_call(
        caller_ura="easynet:///r/example/user/alice",
        callee_ura="easynet:///r/example/agent/alice.worker",
    )
    page = provider.list(
        ReceiptListRequest(
            call=call,
            lookup=ReceiptLookup(trace_id="trace-1"),
            filter=ReceiptFilter(
                caller_ura="easynet:///r/example/user/alice",
                callee_ura="easynet:///r/example/agent/alice.worker",
                subject_uras=(
                    "easynet:///r/example/resource/alice.docs/report",
                    "easynet:///r/example/resource/alice.docs/appendix",
                ),
                ability_uras=("easynet:///r/example/ability/alice.worker.docs.read",),
                state="completed",
                trace_id="trace-1",
            ),
            exclude_ability_uras=(
                "easynet:///r/example/ability/authority.invocation.history.list",
            ),
        )
    )

    assert page.limit == DEFAULT_RECEIPT_PAGE_LIMIT
    assert page.next_cursor == "receipt-history:v1:cursor-1"
    assert page.source.ledger_ura == LEDGER_URA
    assert len(page.records) == 1
    assert isinstance(page.records[0], InvocationLedgerRecord)
    assert page.records[0].receipt_chain.anchors == ()
    assert transport.descriptor_requests[-1]["provider"] == "receipt_history"
    assert transport.descriptor_requests[-1]["subject_ura"] == call.subject_ura
    assert transport.seen["args"] == {
        "key": {"trace_id": "trace-1"},
        "filter": {
            "caller_ura": "easynet:///r/example/user/alice",
            "callee_ura": "easynet:///r/example/agent/alice.worker",
            "ability_ura": "easynet:///r/example/ability/alice.worker.docs.read",
            "state": "completed",
            "trace_id": "trace-1",
            "subject_uras": [
                "easynet:///r/example/resource/alice.docs/report",
                "easynet:///r/example/resource/alice.docs/appendix",
            ],
        },
        "limit": DEFAULT_RECEIPT_PAGE_LIMIT,
        "exclude_ability_uras": [
            "easynet:///r/example/ability/authority.invocation.history.list"
        ],
    }


def test_runtime_receipt_provider_rejects_wrong_device_owner_subject_before_descriptor_resolution() -> None:
    provider, transport = _provider()
    call = _history_call(callee_ura="easynet:///r/example/device/dev-a")
    bad_call = RuntimeCallContext(
        caller_ura=call.caller_ura,
        callee_ura=call.callee_ura,
        subject_ura="easynet:///r/example/device/other-device",
        nonce_base64=call.nonce_base64,
        causal_context=call.causal_context,
        descriptor_version=call.descriptor_version,
        metadata=call.metadata,
        authority=call.authority,
    )

    with pytest.raises(SDKError, match="runtime-state read subject") as caught:
        provider.list(ReceiptListRequest(call=bad_call))
    assert caught.value.code == ErrorCode.INVALID_INVOCATION
    assert transport.descriptor_requests == []


def test_runtime_receipt_provider_accepts_matching_device_owner_subject() -> None:
    provider, transport = _provider()
    device_ura = "easynet:///r/example/device/dev-a"
    call = _history_call(callee_ura=device_ura)
    device_call = RuntimeCallContext(
        caller_ura=call.caller_ura,
        callee_ura=device_ura,
        subject_ura=device_ura,
        nonce_base64=call.nonce_base64,
        causal_context=call.causal_context,
        descriptor_version=call.descriptor_version,
        metadata={},
        authority=DelegationProof.from_metadata(
            _authority_metadata_value(
                {
                    "issuer_ura": call.caller_ura,
                    "subject_ura": device_ura,
                    "caller_ura": call.caller_ura,
                    "audience": device_ura,
                    "scopes": ["invocation.history.*"],
                    "issued_at_ms": 1000,
                    "expires_at_ms": 2000,
                },
                b"delegation-signature",
            )
        ),
    )
    transport.output_json = _output(records=[])

    provider.list(ReceiptListRequest(call=device_call))

    assert transport.descriptor_requests == [
        {
            "callee_ura": device_ura,
            "ability": "invocation.history.list",
            "call_mode": "rpc",
            "caller_ura": call.caller_ura,
            "subject_ura": device_ura,
            "provider": "receipt_history",
        }
    ]


def test_runtime_receipt_provider_rejects_device_owner_subject_with_session_authority() -> None:
    provider, transport = _provider()
    device_ura = "easynet:///r/example/device/dev-a"
    call = _history_call(callee_ura=device_ura)
    device_call = RuntimeCallContext(
        caller_ura=call.caller_ura,
        callee_ura=device_ura,
        subject_ura=device_ura,
        nonce_base64=call.nonce_base64,
        causal_context=call.causal_context,
        descriptor_version=call.descriptor_version,
        metadata={},
        authority=call.authority,
    )

    with pytest.raises(SDKError, match="runtime-owner receipt history subject") as caught:
        provider.list(ReceiptListRequest(call=device_call))
    assert caught.value.code == ErrorCode.AUTHORITY_DENIED
    assert transport.descriptor_requests == []


def test_runtime_receipt_list_accepts_maximum_bound() -> None:
    provider, transport = _provider()
    transport.output_json = _output(records=[])
    page = provider.list(
        ReceiptListRequest(call=_history_call(), limit=MAX_RECEIPT_PAGE_LIMIT)
    )
    assert page.limit == 500
    assert transport.seen["args"] == {"limit": 500}


def test_runtime_receipt_list_projects_multiple_ability_uras_as_one_set() -> None:
    provider, transport = _provider()
    transport.output_json = _output(records=[])
    provider.list(
        ReceiptListRequest(
            call=_history_call(),
            filter=ReceiptFilter(
                ability_uras=(
                    "easynet:///r/example/ability/alice.worker.docs.read",
                    "easynet:///r/example/ability/alice.worker.docs.write",
                )
            ),
        )
    )
    assert transport.seen["args"]["filter"]["ability_uras"] == [
        "easynet:///r/example/ability/alice.worker.docs.read",
        "easynet:///r/example/ability/alice.worker.docs.write",
    ]


@pytest.mark.parametrize("limit", [-1, True, MAX_RECEIPT_PAGE_LIMIT + 1])
def test_runtime_receipt_list_rejects_invalid_page_bound(limit: object) -> None:
    provider, _ = _provider()
    with pytest.raises(SDKError):
        provider.list(ReceiptListRequest(call=_history_call(), limit=limit))  # type: ignore[arg-type]


def test_runtime_receipt_list_forwards_and_validates_cursor() -> None:
    provider, transport = _provider()
    transport.output_json = _output(records=[], next_cursor="receipt-history:v1:cursor-2")
    page = provider.list(
        ReceiptListRequest(
            call=_history_call(),
            cursor=" receipt-history:v1:cursor-1 ",
            limit=2,
        )
    )
    assert transport.seen["args"] == {
        "limit": 2,
        "cursor": "receipt-history:v1:cursor-1",
    }
    assert page.next_cursor == "receipt-history:v1:cursor-2"

    transport.output_json = _output(records=[], next_cursor="receipt-history:v1:cursor-1")
    with pytest.raises(SDKError, match="repeated cursor"):
        provider.list(
            ReceiptListRequest(call=_history_call(), cursor="receipt-history:v1:cursor-1")
        )

    with pytest.raises(SDKError, match="cursor exceeds"):
        provider.list(ReceiptListRequest(call=_history_call(), cursor="x" * 4097))


def test_runtime_receipt_list_rejects_noncanonical_and_duplicate_ura_filters() -> None:
    provider, transport = _provider()
    with pytest.raises(SDKError, match="caller_ura must be a canonical URA"):
        provider.list(
            ReceiptListRequest(
                call=_history_call(),
                filter=ReceiptFilter(caller_ura="https://example.invalid/user/alice"),
            )
        )
    with pytest.raises(SDKError, match="must not contain duplicates"):
        provider.list(
            ReceiptListRequest(
                call=_history_call(),
                exclude_ability_uras=(
                    "easynet:///r/example/ability/authority.observe.health",
                    "easynet:///r/example/ability/authority.observe.health",
                ),
            )
        )
    assert transport.seen == {}


def test_runtime_receipt_list_fails_closed_without_stable_cursor() -> None:
    provider, transport = _provider()
    transport.output_json = _output(
        records=[_record(request_id="one"), _record(request_id="two")]
    )
    with pytest.raises(SDKError, match="exceeds the bounded page"):
        provider.list(ReceiptListRequest(call=_history_call(), limit=1))


def test_runtime_receipt_list_requires_canonical_ledger_source() -> None:
    provider, transport = _provider()
    transport.output_json = _output(
        ledger_ura="https://example.invalid/invocations",
        records=[],
    )
    with pytest.raises(SDKError, match="ledger_ura must be a canonical URA"):
        provider.list(ReceiptListRequest(call=_history_call()))


def test_runtime_receipt_get_requires_exactly_one_lookup_key() -> None:
    provider, _ = _provider()
    with pytest.raises(SDKError, match="exactly one Receipt lookup key"):
        provider.get(
            ReceiptGetRequest(
                call=_call(),
                lookup=ReceiptLookup(request_id="request-1", trace_id="trace-1"),
            )
        )


def test_runtime_receipt_get_preserves_explicit_not_found_result() -> None:
    provider, transport = _provider()
    transport.output_json = _output(record=None)
    result = provider.get(
        ReceiptGetRequest(
            call=_history_call(
                scope="invocation.history.get",
                followup_ability="invocation.history.get",
            ),
            lookup=ReceiptLookup(request_id="request-1"),
        )
    )
    assert result.record is None
    assert result.source.ledger_ura == LEDGER_URA
    assert transport.seen["args"] == {"key": {"request_id": "request-1"}}


def test_runtime_receipt_get_rejects_missing_record_projection() -> None:
    provider, transport = _provider()
    transport.output_json = _output()
    with pytest.raises(SDKError, match="must include record"):
        provider.get(
            ReceiptGetRequest(
                call=_history_call(
                    scope="invocation.history.get",
                    followup_ability="invocation.history.get",
                ),
                lookup=ReceiptLookup(request_id="request-1"),
            )
        )


def test_runtime_receipt_trace_normalizes_daemon_nodes_through_axon_parser() -> None:
    provider, transport = _provider()
    transport.output_json = _output(
        trace_id="trace-1",
        nodes=[_record()],
        edges=[],
        edge_semantics="daemon transport metadata",
    )
    result = provider.trace(
        ReceiptTraceRequest(
            call=_history_call(
                scope="invocation.trace.get",
                followup_ability="invocation.trace.get",
            ),
            lookup=ReceiptLookup(invocation_ura=_record()["invocation_ura"]),
        )
    )
    assert result.graph.trace_id == "trace-1"
    assert len(result.graph.records) == 1
    assert result.graph.records[0].request_id == "request-1"
    assert result.graph.edges == ()
    assert transport.seen["args"] == {"key": {"ura": _record()["invocation_ura"]}}


def test_runtime_receipt_rejects_malformed_axon_record() -> None:
    provider, transport = _provider()
    transport.output_json = _output(records=[{"request_id": "missing-uras"}])
    with pytest.raises(SDKError, match="invalid Axon invocation ledger projection"):
        provider.list(ReceiptListRequest(call=_history_call()))


def test_receipt_client_delegates_verification_to_axon_receipt() -> None:
    client = ReceiptClient(_ProviderFake())
    resolver = object()
    receipt = _ReceiptFake()
    assert client.verify(receipt, resolver) == "verified"  # type: ignore[arg-type]
    assert receipt.resolver is resolver
    with pytest.raises(SDKError, match="key resolver"):
        client.verify(receipt, None)  # type: ignore[arg-type]


def test_receipt_client_delegates_chain_verification_to_axon(monkeypatch) -> None:
    seen: list[object] = []
    expected = object()

    def verify(receipts: list[object]) -> object:
        seen.extend(receipts)
        return expected

    monkeypatch.setattr(receipt_module, "verify_receipt_chain", verify)
    first, second = object(), object()
    client = ReceiptClient(_ProviderFake())
    assert client.verify_chain([first, second]) is expected  # type: ignore[list-item]
    assert seen == [first, second]


def test_receipt_facade_requires_providers() -> None:
    with pytest.raises(SDKError, match="Receipt provider is required"):
        ReceiptClient(None)  # type: ignore[arg-type]
    with pytest.raises(SDKError, match="runtime ability client is required"):
        RuntimeReceiptProvider(None)  # type: ignore[arg-type]


class _ProviderFake:
    def list(self, request):
        raise AssertionError(request)

    def get(self, request):
        raise AssertionError(request)

    def trace(self, request):
        raise AssertionError(request)


class _ReceiptFake:
    resolver: object | None = None

    def verify(self, resolver: object) -> str:
        self.resolver = resolver
        return "verified"
