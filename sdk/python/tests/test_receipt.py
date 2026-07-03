import json
import unittest

from easynet_sdk import ErrorCode, ReceiptClient, ReceiptFetchRequest, ReceiptSummary, SDKError


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
        self.causal_ref_json = (
            b'{"causal_ref":"receipt:easynet:///r/example/receipt/receipt-1",'
            b'"receipt_ura":"easynet:///r/example/receipt/receipt-1",'
            b'"invocation_id":"inv-example-1","form":"scalar","metadata":{}}'
        )
        self.seen_request: dict[str, object] | None = None
        self.seen_receipt_raw = b""

    def fetch(self, request_json: bytes) -> bytes:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        return self.fetch_json

    def project(self, receipt_json: bytes) -> bytes:
        self.seen_receipt_raw = receipt_json
        return self.project_json

    def verify(self, receipt_json: bytes) -> bytes:
        self.seen_receipt_raw = receipt_json
        return self.verify_json

    def causal_ref(self, receipt_json: bytes) -> bytes:
        self.seen_receipt_raw = receipt_json
        return self.causal_ref_json


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


if __name__ == "__main__":
    unittest.main()
