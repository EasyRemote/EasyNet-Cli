import json
import unittest

from easynet_sdk import (
    ErrorCode,
    ReceiptChainVerificationRequest,
    ReceiptClient,
    ReceiptFetchRequest,
    ReceiptSummary,
    SDKError,
    is_code,
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
            b'{"causal_ref":"receipt:easynet:///r/example/receipt/receipt-1",'
            b'"receipt_ura":"easynet:///r/example/receipt/receipt-1",'
            b'"invocation_id":"inv-example-1","form":"scalar","metadata":{}}'
        )
        self.seen_request: dict[str, object] | None = None
        self.seen_chain_request: dict[str, object] | None = None
        self.seen_receipt_raw = b""
        self.close_calls = 0

    def fetch(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        return self.fetch_json

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

    def test_fetch_rejects_missing_or_ambiguous_lookup_key(self) -> None:
        transport = MemoryReceiptTransport()
        client = ReceiptClient(transport)
        request = fetch_request()

        with self.assertRaises(SDKError):
            client.fetch(
                ReceiptFetchRequest(
                    caller_ura=request.caller_ura,
                    callee_ura=request.callee_ura,
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
        causal = client.causal_ref(b'{"receipt_ura":"easynet:///r/example/receipt/receipt-1"}')

        self.assertTrue(verification.verified)
        self.assertEqual(verification.method, "axon-full-receipt")
        self.assertEqual(causal.form, "scalar")
        self.assertTrue(causal.causal_ref)

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
