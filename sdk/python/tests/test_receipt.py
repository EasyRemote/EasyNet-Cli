from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Any

import pytest

import easynet_sdk.receipt as receipt_module
from easynet_sdk._receipt_routes import _RECEIPT_ROUTE_MANIFEST_SHA256
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
    _RuntimeReceiptRouteSet,
)
from easynet_sdk.runtime import RuntimeClient
from easynet_sdk.runtime_ability import RuntimeAbilityClient

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
    transport = RuntimeTransportFake()
    ability = RuntimeAbilityClient(
        RuntimeClient(transport),  # type: ignore[arg-type]
        AddressingClient(AxonAddressingTransport()),
    )
    return RuntimeReceiptProvider(ability), transport


def _output(**values: object) -> dict[str, object]:
    return {
        "ledger_ura": LEDGER_URA,
        **values,
    }


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
    page = provider.list(
        ReceiptListRequest(
            call=_call(),
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


def test_runtime_receipt_provider_uses_explicit_route_set() -> None:
    class RouteAwareTransport(RuntimeTransportFake):
        def resolve_descriptor_ref(self, request_json: bytes) -> bytes:
            import json

            request = json.loads(request_json)
            return json.dumps(
                {
                    "descriptor_ref": (
                        "easynet:///r/example/ability/authority."
                        f"{request['ability']}@1.0.0"
                    )
                }
            ).encode()

    transport = RouteAwareTransport()
    ability = RuntimeAbilityClient(
        RuntimeClient(transport),  # type: ignore[arg-type]
        AddressingClient(AxonAddressingTransport()),
    )
    provider = RuntimeReceiptProvider(
        ability,
        routes=_RuntimeReceiptRouteSet(
            "receipt.catalog.list",
            "receipt.catalog.get",
            "receipt.catalog.trace",
        ),
    )

    transport.output_json = _output(records=[])
    provider.list(ReceiptListRequest(call=_call()))
    assert transport.seen["descriptor_ref"] == (
        "easynet:///r/example/ability/authority.receipt.catalog.list@1.0.0"
    )

    transport.output_json = _output(record=None)
    provider.get(
        ReceiptGetRequest(call=_call(), lookup=ReceiptLookup(request_id="request-1"))
    )
    assert transport.seen["descriptor_ref"] == (
        "easynet:///r/example/ability/authority.receipt.catalog.get@1.0.0"
    )

    transport.output_json = _output(trace_id="trace-1", nodes=[], edges=[])
    provider.trace(
        ReceiptTraceRequest(call=_call(), lookup=ReceiptLookup(trace_id="trace-1"))
    )
    assert transport.seen["descriptor_ref"] == (
        "easynet:///r/example/ability/authority.receipt.catalog.trace@1.0.0"
    )


def test_runtime_receipt_provider_rejects_incomplete_route_set() -> None:
    with pytest.raises(SDKError, match="runtime receipt get route ability is required"):
        _RuntimeReceiptRouteSet(
            "receipt.catalog.list",
            "",
            "receipt.catalog.trace",
        )


def test_runtime_receipt_list_accepts_maximum_bound() -> None:
    provider, transport = _provider()
    transport.output_json = _output(records=[])
    page = provider.list(ReceiptListRequest(call=_call(), limit=MAX_RECEIPT_PAGE_LIMIT))
    assert page.limit == 500
    assert transport.seen["args"] == {"limit": 500}


def test_runtime_receipt_list_projects_multiple_ability_uras_as_one_set() -> None:
    provider, transport = _provider()
    transport.output_json = _output(records=[])
    provider.list(
        ReceiptListRequest(
            call=_call(),
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
        provider.list(ReceiptListRequest(call=_call(), limit=limit))  # type: ignore[arg-type]


def test_runtime_receipt_list_forwards_and_validates_cursor() -> None:
    provider, transport = _provider()
    transport.output_json = _output(records=[], next_cursor="receipt-history:v1:cursor-2")
    page = provider.list(
        ReceiptListRequest(
            call=_call(),
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
            ReceiptListRequest(call=_call(), cursor="receipt-history:v1:cursor-1")
        )

    with pytest.raises(SDKError, match="cursor exceeds"):
        provider.list(ReceiptListRequest(call=_call(), cursor="x" * 4097))


def test_runtime_receipt_list_rejects_noncanonical_and_duplicate_ura_filters() -> None:
    provider, transport = _provider()
    with pytest.raises(SDKError, match="caller_ura must be a canonical URA"):
        provider.list(
            ReceiptListRequest(
                call=_call(),
                filter=ReceiptFilter(caller_ura="https://example.invalid/user/alice"),
            )
        )
    with pytest.raises(SDKError, match="must not contain duplicates"):
        provider.list(
            ReceiptListRequest(
                call=_call(),
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
        provider.list(ReceiptListRequest(call=_call(), limit=1))


def test_runtime_receipt_list_requires_canonical_ledger_source() -> None:
    provider, transport = _provider()
    transport.output_json = _output(
        ledger_ura="https://example.invalid/invocations",
        records=[],
    )
    with pytest.raises(SDKError, match="ledger_ura must be a canonical URA"):
        provider.list(ReceiptListRequest(call=_call()))


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
            call=_call(),
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
                call=_call(),
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
            call=_call(),
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
        provider.list(ReceiptListRequest(call=_call()))


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
