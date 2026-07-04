import json
import unittest
from pathlib import Path

from easynet_sdk import (
    AbilityCallRequest,
    InvocationResult,
    ErrorCode,
    LocalReceiptTransport,
    ReceiptChain,
    ReceiptChainVerificationRequest,
    ReceiptClient,
    ReceiptFetchRequest,
    ReceiptRef,
    ReceiptSummary,
    ReceiptVerification,
    SDKError,
    build_receipt_fetch_invocation,
    is_code,
)

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = ROOT / "sdk/conformance/fixtures"


def shared_fixture(name: str) -> bytes:
    return (FIXTURES / name).read_bytes()


def assert_json_equivalent(actual: bytes, expected: bytes) -> None:
    if json.loads(actual.decode("utf-8")) != json.loads(expected.decode("utf-8")):
        raise AssertionError(
            "JSON mismatch\n"
            f"actual: {actual.decode('utf-8')}\n"
            f"expected: {expected.decode('utf-8')}"
        )


class MemoryReceiptTransport:
    def __init__(self) -> None:
        self.fetch_json = (
            b'{"receipt_ura":null,"invocation_id":"inv-example-1",'
            b'"state":"completed","verified":false,"output":{"ok":true},'
            b'"error":null,"causal_ref":null,"metadata":{}}'
        )
        self.project_json = self.fetch_json
        self.verify_json = (
            b'{"verified":true,"receipt_ura":"easynet:///r/example/receipt/receipt-1",'
            b'"invocation_id":"inv-example-1","method":"axon-full-receipt",'
            b'"metadata":{"source":"axon"}}'
        )
        self.verify_chain_json = (
            b'{"verified":false,"continuous":true,'
            b'"method":"daemon_receipt_chain_continuity","reason":"continuity only",'
            b'"requires_full_receipt":true,'
            b'"root_receipt_ura":"easynet:///r/example/receipt/receipt-1",'
            b'"terminal_receipt_ura":"easynet:///r/example/receipt/receipt-2",'
            b'"receipt_count":2,'
            b'"items":[{"index":0,'
            b'"receipt_ura":"easynet:///r/example/receipt/receipt-1",'
            b'"receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",'
            b'"prev_receipt_hash_hex":null,"continuous":true,"metadata":{}},'
            b'{"index":1,"receipt_ura":"easynet:///r/example/receipt/receipt-2",'
            b'"receipt_hash_hex":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",'
            b'"prev_receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",'
            b'"continuous":true,"metadata":{}}],'
            b'"metadata":{"chain_projection":"hash_continuity"}}'
        )
        self.causal_ref_json = (
            b'{"receipt_ura":"easynet:///r/example/receipt/receipt-1",'
            b'"receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",'
            b'"verified":false,'
            b'"causal_context":{"form":"scalar",'
            b'"receipt_ura":"easynet:///r/example/receipt/receipt-1",'
            b'"receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},'
            b'"causal_ref":"receipt:easynet:///r/example/receipt/receipt-1",'
            b'"invocation_id":"inv-example-1","form":"scalar","metadata":{}}'
        )
        self.fetch_invocation_json = shared_fixture("receipt-fetch-invocation.v4.json")
        self.seen_request: dict[str, object] | None = None
        self.seen_fetch_invocation_request: dict[str, object] | None = None
        self.seen_chain_request: dict[str, object] | None = None
        self.seen_receipt_raw = b""
        self.close_calls = 0

    def fetch(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        return self.fetch_json

    def build_fetch_invocation(self, request_json: bytes) -> bytes:
        self.seen_fetch_invocation_request = json.loads(request_json.decode("utf-8"))
        return self.fetch_invocation_json

    def project(self, receipt_json: bytes) -> bytes:
        self.seen_receipt_raw = receipt_json
        return self.project_json

    def verify(self, receipt_json: bytes) -> bytes:
        self.seen_receipt_raw = receipt_json
        return self.verify_json

    def verify_chain(self, request_json: bytes) -> bytes:
        self.seen_chain_request = json.loads(request_json.decode("utf-8"))
        return self.verify_chain_json

    def causal_ref(self, receipt_json: bytes) -> bytes:
        self.seen_receipt_raw = receipt_json
        return self.causal_ref_json

    def close(self) -> None:
        self.close_calls += 1


def fetch_request() -> ReceiptFetchRequest:
    return ReceiptFetchRequest(
        caller_ura="easynet:///r/example/agent/alice.sdk",
        callee_ura="easynet:///r/example/device/dev-a",
        descriptor_ref="easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
        subject_ura="easynet:///r/example/device/dev-a",
        descriptor_version="1.0.0",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        request_id="inv-example-1",
        metadata={"request_id": "receipt-fetch-1"},
    )


class ReceiptTests(unittest.TestCase):
    def test_fetch_preserves_carrier_and_decodes_summary(self) -> None:
        transport = MemoryReceiptTransport()
        client = ReceiptClient(transport)

        summary = client.fetch(fetch_request())

        self.assertEqual(summary.state, "completed")
        self.assertFalse(summary.verified)
        self.assertEqual(summary.invocation_id, "inv-example-1")
        assert transport.seen_request is not None
        self.assertEqual(transport.seen_request["request_id"], "inv-example-1")
        self.assertEqual(
            transport.seen_request["caller_ura"],
            "easynet:///r/example/agent/alice.sdk",
        )

    def test_build_fetch_invocation_matches_shared_carrier(self) -> None:
        decoded = json.loads(shared_fixture("receipt-fetch-request.v4.json"))
        draft = build_receipt_fetch_invocation(
            ReceiptFetchRequest(
                caller_ura=decoded["caller_ura"],
                callee_ura=decoded["callee_ura"],
                descriptor_ref=decoded["descriptor_ref"],
                subject_ura=decoded["subject_ura"],
                descriptor_version=decoded["descriptor_version"],
                nonce_base64=decoded["nonce_base64"],
                causal_context=decoded["causal_context"],
                request_id=decoded["request_id"],
                metadata=decoded["metadata"],
            )
        )

        assert_json_equivalent(
            draft.to_json().encode("utf-8"),
            shared_fixture("receipt-fetch-invocation.v4.json"),
        )

    def test_client_build_fetch_invocation_delegates_to_transport(self) -> None:
        transport = MemoryReceiptTransport()
        client = ReceiptClient(transport)

        draft = client.build_fetch_invocation(fetch_request())

        assert transport.seen_fetch_invocation_request is not None
        self.assertEqual(
            transport.seen_fetch_invocation_request["request_id"],
            "inv-example-1",
        )
        assert_json_equivalent(
            draft.to_json().encode("utf-8"),
            shared_fixture("receipt-fetch-invocation.v4.json"),
        )

    def test_client_build_fetch_invocation_honors_lifecycle(self) -> None:
        transport = MemoryReceiptTransport()
        client = ReceiptClient(transport)

        client.close()

        with self.assertRaises(SDKError) as caught:
            client.build_fetch_invocation(fetch_request())
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_fetch_rejects_missing_or_ambiguous_lookup_key(self) -> None:
        transport = MemoryReceiptTransport()
        client = ReceiptClient(transport)
        request = fetch_request()

        with self.assertRaises(SDKError):
            client.fetch(
                ReceiptFetchRequest(
                    caller_ura=request.caller_ura,
                    callee_ura=request.callee_ura,
                    descriptor_ref=request.descriptor_ref,
                    subject_ura=request.subject_ura,
                    descriptor_version=request.descriptor_version,
                    nonce_base64=request.nonce_base64,
                    causal_context=request.causal_context,
                )
            )

        with self.assertRaises(SDKError):
            client.fetch(
                ReceiptFetchRequest(
                    caller_ura=request.caller_ura,
                    callee_ura=request.callee_ura,
                    descriptor_ref=request.descriptor_ref,
                    subject_ura=request.subject_ura,
                    descriptor_version=request.descriptor_version,
                    nonce_base64=request.nonce_base64,
                    causal_context=request.causal_context,
                    request_id="inv-example-1",
                    trace_id="trace-1",
                )
            )

        self.assertIsNone(transport.seen_request)

    def test_project_does_not_upgrade_verification(self) -> None:
        transport = MemoryReceiptTransport()
        client = ReceiptClient(transport)

        summary = client.project(b'{"raw":true}')

        self.assertFalse(summary.verified)
        self.assertEqual(transport.seen_receipt_raw, b'{"raw":true}')

    def test_verify_and_causal_ref_decode_daemon_projections(self) -> None:
        transport = MemoryReceiptTransport()
        client = ReceiptClient(transport)

        verification = client.verify(b'{"receipt_ura":"easynet:///r/example/receipt/receipt-1"}')
        causal = client.causal_ref(
            b'{"receipt_ura":"easynet:///r/example/receipt/receipt-1",'
            b'"receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}'
        )

        self.assertTrue(verification.verified)
        self.assertEqual(verification.method, "axon-full-receipt")
        self.assertTrue(verification.is_cryptographic)
        self.assertIs(verification.require_cryptographic(), verification)
        self.assertEqual(causal.form, "scalar")
        self.assertEqual(
            causal.to_causal_context(),
            {
                "form": "scalar",
                "receipt_ura": "easynet:///r/example/receipt/receipt-1",
                "receipt_hash_hex": (
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                ),
            },
        )
        self.assertEqual(
            client.causal_context(
                b'{"receipt_ura":"easynet:///r/example/receipt/receipt-1",'
                b'"receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}'
            ),
            causal.to_causal_context(),
        )

    def test_causal_context_from_runtime_receipt_and_invocation_result(self) -> None:
        transport = MemoryReceiptTransport()
        client = ReceiptClient(transport)
        result = InvocationResult.from_json(
            json.dumps(
                {
                    "ok": True,
                    "tuple": {
                        "caller_ura": "easynet:///r/example/agent/alice.sdk",
                        "callee_ura": "easynet:///r/example/device/dev-a",
                        "descriptor_ref": (
                            "easynet:///r/example/ability/"
                            "device.dev-a.observe.health@1.0.0"
                        ),
                        "subject_ura": "easynet:///r/example/device/dev-a",
                        "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                        "causal_context": {"form": "none"},
                        "content_type": "application/json",
                        "args": {},
                    },
                    "terminal_state": "Completed",
                    "output_content_type": "application/json",
                    "output_base64": "e30=",
                    "output_json": {},
                    "elapsed_ms": 8,
                    "receipt": {
                        "receipt_ura": "easynet:///r/example/receipt/receipt-1",
                        "invocation_id": "inv-example-1",
                        "self_hash_hex": (
                            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        ),
                    },
                    "error": None,
                },
                separators=(",", ":"),
                sort_keys=True,
            )
        )

        assert result.receipt_summary is not None
        causal_context = client.causal_context_from_runtime_receipt(
            result.receipt_summary
        )

        self.assertEqual(
            causal_context,
            {
                "form": "scalar",
                "receipt_ura": "easynet:///r/example/receipt/receipt-1",
                "receipt_hash_hex": (
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                ),
            },
        )
        self.assertEqual(
            client.causal_context_from_invocation_result(result),
            causal_context,
        )
        child = AbilityCallRequest(
            caller_ura="easynet:///r/example/agent/alice.sdk",
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="easynet:///r/example/device/dev-a",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context=causal_context,
            ability_name="child.echo",
            args={},
        )
        self.assertEqual(child.causal_context["form"], "scalar")

    def test_causal_context_rejects_unanchored_runtime_result(self) -> None:
        transport = MemoryReceiptTransport()
        client = ReceiptClient(transport)
        result = InvocationResult.from_json(
            json.dumps(
                {
                    "ok": True,
                    "tuple": build_receipt_fetch_invocation(
                        fetch_request()
                    ).to_json_dict(),
                    "terminal_state": "Completed",
                    "receipt": {"invocation_id": "inv-example-1"},
                    "error": None,
                },
                separators=(",", ":"),
                sort_keys=True,
            )
        )

        with self.assertRaises(SDKError):
            client.causal_context_from_invocation_result(result)

    def test_verify_chain_preserves_receipt_bodies_and_decodes_continuity(self) -> None:
        transport = MemoryReceiptTransport()
        client = ReceiptClient(transport)

        result = client.verify_chain(
            ReceiptChainVerificationRequest(
                receipts=(
                    b'{"receipt_ura":"easynet:///r/example/receipt/receipt-1",'
                    b'"self_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}',
                    b'{"receipt_ura":"easynet:///r/example/receipt/receipt-2",'
                    b'"self_hash_hex":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",'
                    b'"prev_receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}',
                ),
                metadata={"request_id": "chain-1"},
            )
        )

        self.assertFalse(result.verified)
        self.assertTrue(result.continuous)
        self.assertEqual(result.method, "daemon_receipt_chain_continuity")
        self.assertEqual(result.receipt_count, 2)
        assert transport.seen_chain_request is not None
        receipts = transport.seen_chain_request["receipts"]
        self.assertIsInstance(receipts, list)
        self.assertEqual(
            receipts[0]["receipt_ura"],
            "easynet:///r/example/receipt/receipt-1",
        )

    def test_verify_chain_rejects_duplicate_receipt_hash(self) -> None:
        transport = MemoryReceiptTransport()
        client = ReceiptClient(transport)

        with self.assertRaises(SDKError):
            client.verify_chain(
                ReceiptChainVerificationRequest(
                    receipts=(
                        b'{"receipt_ura":"easynet:///r/example/receipt/receipt-1",'
                        b'"self_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}',
                        b'{"receipt_ura":"easynet:///r/example/receipt/receipt-2",'
                        b'"receipt_hash":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}',
                    )
                )
            )

        self.assertIsNone(transport.seen_chain_request)

    def test_receipt_ref_delegates_causal_context_projection(self) -> None:
        transport = MemoryReceiptTransport()
        client = ReceiptClient(transport)
        ref = ReceiptRef(
            receipt_ura=" easynet:///r/example/receipt/receipt-1 ",
            receipt_hash_hex=(
                "sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            ),
            invocation_id="inv-example-1",
            metadata={"source": "runtime"},
        )

        causal_context = ref.causal_context(client)

        self.assertEqual(ref.receipt_ura, "easynet:///r/example/receipt/receipt-1")
        self.assertEqual(
            ref.receipt_hash_hex,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        self.assertEqual(
            json.loads(transport.seen_receipt_raw.decode("utf-8")),
            {
                "invocation_id": "inv-example-1",
                "metadata": {"source": "runtime"},
                "receipt_hash_hex": (
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                ),
                "receipt_ura": "easynet:///r/example/receipt/receipt-1",
            },
        )
        self.assertEqual(causal_context["form"], "scalar")

    def test_receipt_ref_from_runtime_receipt_requires_anchor(self) -> None:
        anchored = ReceiptRef.from_runtime_receipt(
            InvocationResult.from_json(
                json.dumps(
                    {
                        "ok": True,
                        "tuple": build_receipt_fetch_invocation(
                            fetch_request()
                        ).to_json_dict(),
                        "terminal_state": "Completed",
                        "receipt": {
                            "receipt_ura": "easynet:///r/example/receipt/receipt-1",
                            "invocation_id": "inv-example-1",
                            "self_hash_hex": (
                                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            ),
                        },
                        "error": None,
                    },
                    separators=(",", ":"),
                    sort_keys=True,
                )
            ).receipt_summary
        )

        self.assertEqual(anchored.invocation_id, "inv-example-1")

        unanchored = InvocationResult.from_json(
            json.dumps(
                {
                    "ok": True,
                    "tuple": build_receipt_fetch_invocation(fetch_request()).to_json_dict(),
                    "terminal_state": "Completed",
                    "receipt": {"invocation_id": "inv-example-1"},
                    "error": None,
                },
                separators=(",", ":"),
                sort_keys=True,
            )
        ).receipt_summary
        assert unanchored is not None
        with self.assertRaises(SDKError):
            ReceiptRef.from_runtime_receipt(unanchored)

        with self.assertRaises(SDKError):
            ReceiptRef.from_mapping(
                {
                    "receipt_ura": "easynet:///r/example/receipt/receipt-1",
                    "receipt_hash_hex": "aa",
                }
            )

    def test_receipt_chain_delegates_continuity_projection_to_client(self) -> None:
        transport = MemoryReceiptTransport()
        client = ReceiptClient(transport)
        chain = ReceiptChain.from_mappings(
            (
                {
                    "receipt_ura": "easynet:///r/example/receipt/receipt-1",
                    "receipt_hash": (
                        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    ),
                    "index": 0,
                },
                {
                    "receipt_ura": "easynet:///r/example/receipt/receipt-2",
                    "self_hash_hex": (
                        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    ),
                    "prev_receipt_hash_hex": (
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    ),
                    "index": 1,
                },
            )
        )

        verification = chain.verify_continuity(
            client, metadata={"request_id": "chain-ref-1"}
        )

        self.assertTrue(verification.continuous)
        assert transport.seen_chain_request is not None
        self.assertEqual(
            transport.seen_chain_request["metadata"]["request_id"], "chain-ref-1"
        )
        receipts = transport.seen_chain_request["receipts"]
        self.assertEqual(receipts[0]["receipt_hash_hex"], "a" * 64)
        self.assertEqual(receipts[1]["prev_receipt_hash_hex"], "a" * 64)

    def test_local_receipt_transport_projects_summary_continuity(self) -> None:
        client = ReceiptClient(LocalReceiptTransport())
        chain = ReceiptChainVerificationRequest(
            receipts=(
                b'{"invocation_id":"inv-1","self_hash_hex":"'
                + b"a" * 64
                + b'"}',
                b'{"invocation_id":"inv-2","self_hash_hex":"'
                + b"b" * 64
                + b'","prev_receipt_hash_hex":"'
                + b"a" * 64
                + b'"}',
            )
        )

        verification = client.verify_chain(chain)
        summary = client.project(
            b'{"invocation_id":"inv-1","state":"completed","self_hash_hex":"'
            + b"a" * 64
            + b'"}'
        )
        check = client.verify(
            b'{"invocation_id":"inv-1","self_hash_hex":"' + b"a" * 64 + b'"}'
        )

        self.assertTrue(verification.continuous)
        self.assertFalse(verification.verified)
        self.assertTrue(verification.requires_full_receipt)
        self.assertEqual(verification.items[0].receipt_ura, "summary:inv-1:0")
        self.assertEqual(summary.metadata["source"], "sdk_local_receipt")
        self.assertFalse(check.verified)
        self.assertEqual(check.method, "summary-only")

    def test_local_receipt_transport_reports_broken_summary_chain(self) -> None:
        client = ReceiptClient(LocalReceiptTransport())

        verification = client.verify_chain(
            ReceiptChainVerificationRequest(
                receipts=(
                    b'{"invocation_id":"inv-1","self_hash_hex":"'
                    + b"a" * 64
                    + b'"}',
                    b'{"invocation_id":"inv-2","self_hash_hex":"'
                    + b"b" * 64
                    + b'","prev_receipt_hash_hex":"'
                    + b"c" * 64
                    + b'"}',
                )
            )
        )

        self.assertFalse(verification.continuous)
        self.assertFalse(verification.verified)
        self.assertIn("index 1", verification.reason)
        self.assertFalse(verification.items[1].continuous)

    def test_local_receipt_transport_requires_receipt_ura_for_causal_ref(self) -> None:
        client = ReceiptClient(LocalReceiptTransport())

        with self.assertRaises(SDKError):
            client.causal_ref(
                b'{"invocation_id":"inv-1","self_hash_hex":"' + b"a" * 64 + b'"}'
            )

        causal = client.causal_ref(
            b'{"receipt_ura":"easynet:///r/example/receipt/receipt-1",'
            b'"invocation_id":"inv-1","self_hash_hex":"'
            + b"a" * 64
            + b'"}'
        )
        self.assertEqual(causal.causal_context["form"], "scalar")

    def test_causal_ref_rejects_empty_projection(self) -> None:
        transport = MemoryReceiptTransport()
        transport.causal_ref_json = b'{"metadata":{}}'
        client = ReceiptClient(transport)

        with self.assertRaises(SDKError):
            client.causal_ref(b'{"receipt":true}')

    def test_summary_decodes_typed_error_and_null_output(self) -> None:
        summary = ReceiptSummary.from_json(
            b'{"state":"failed","verified":false,"output":null,'
            b'"error":{"code":"InvalidArgument","stage":"runtime",'
            b'"message":"bad receipt","retry":"never",'
            b'"details":{"field":"receipt_ura"}},"metadata":{}}'
        )

        self.assertIsNone(summary.output)
        assert summary.error is not None
        self.assertEqual(summary.error.code, ErrorCode.INVALID_ARGUMENT)
        self.assertEqual(summary.error.stage, "runtime")
        self.assertEqual(summary.error.details["field"], "receipt_ura")

    def test_summary_requires_output_field(self) -> None:
        with self.assertRaises(SDKError):
            ReceiptSummary.from_json(b'{"state":"completed","verified":false,"metadata":{}}')

    def test_verification_assurance_rejects_summary_only_claims(self) -> None:
        summary_only = ReceiptVerification.from_json(
            b'{"verified":false,"method":"summary-only",'
            b'"reason":"full receipt required",'
            b'"metadata":{"source":"sdk_conformance"}}'
        )
        continuity_only = ReceiptVerification.from_json(
            b'{"verified":true,"method":"daemon_receipt_chain_continuity",'
            b'"metadata":{"source":"daemon"}}'
        )
        metadata_backed = ReceiptVerification.from_json(
            b'{"verified":true,"method":"full-receipt",'
            b'"metadata":{"assurance":"cryptographic"}}'
        )

        self.assertFalse(summary_only.is_cryptographic)
        self.assertFalse(continuity_only.is_cryptographic)
        self.assertTrue(metadata_backed.is_cryptographic)
        self.assertIs(metadata_backed.require_cryptographic(), metadata_backed)
        with self.assertRaises(SDKError):
            summary_only.require_cryptographic()
        with self.assertRaises(SDKError):
            continuity_only.require_cryptographic()
        with self.assertRaises(SDKError):
            ReceiptVerification.from_json(
                b'{"verified":true,"method":"summary-only","metadata":{}}'
            )

    def test_close_delegates_once_and_fails_closed(self) -> None:
        transport = MemoryReceiptTransport()
        client = ReceiptClient(transport)

        client.close()
        client.close()

        self.assertEqual(transport.close_calls, 1)
        with self.assertRaises(SDKError) as caught:
            client.fetch(fetch_request())
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIsNone(transport.seen_request)


if __name__ == "__main__":
    unittest.main()
