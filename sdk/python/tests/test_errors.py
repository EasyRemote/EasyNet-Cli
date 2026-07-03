import unittest

from easynet_sdk import ErrorCode, SDKError, is_code


class ErrorTests(unittest.TestCase):
    def test_from_json_decodes_fixture_shape(self) -> None:
        error = SDKError.from_json(
            b"""{
                "code": "InvalidArgument",
                "stage": "prepare",
                "message": "missing caller_ura",
                "retry": "never",
                "source": "sdk",
                "invocation_id": null,
                "receipt_ura": null,
                "details": {}
            }"""
        )

        self.assertIsNotNone(error)
        assert error is not None
        self.assertEqual(error.code, ErrorCode.INVALID_ARGUMENT)
        self.assertFalse(error.retryable)
        self.assertEqual(error.source, "sdk")

    def test_from_json_preserves_runtime_refs_and_retryability(self) -> None:
        error = SDKError.from_json(
            b"""{
                "code": "TIMEOUT",
                "stage": "transport",
                "message": "deadline elapsed",
                "retry": "safe",
                "source": "c_abi",
                "invocation_id": "inv-1",
                "receipt_ura": "easynet:///r/example/receipt/opaque",
                "details": {"abi_symbol": "ERR_TIMEOUT"}
            }"""
        )

        self.assertIsNotNone(error)
        assert error is not None
        self.assertEqual(error.code, ErrorCode.TIMEOUT)
        self.assertTrue(error.retryable)
        self.assertEqual(error.invocation_id, "inv-1")
        self.assertEqual(error.receipt_ura, "easynet:///r/example/receipt/opaque")
        self.assertEqual(error.details["abi_symbol"], "ERR_TIMEOUT")

    def test_from_json_rejects_invalid_retry_hint(self) -> None:
        with self.assertRaises(SDKError) as caught:
            SDKError.from_json(
                b"""{
                    "code": "TIMEOUT",
                    "stage": "transport",
                    "message": "deadline elapsed",
                    "retry": "maybe",
                    "details": {}
                }"""
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_from_json_rejects_invalid_utf8_as_sdk_error(self) -> None:
        with self.assertRaises(SDKError) as caught:
            SDKError.from_json(b"\xff")

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_from_json_null_is_no_error(self) -> None:
        self.assertIsNone(SDKError.from_json(b"null"))


if __name__ == "__main__":
    unittest.main()
