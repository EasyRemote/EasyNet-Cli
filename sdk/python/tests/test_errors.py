import unittest

from easynet_sdk import ErrorCode, RetryHint, SDKError, is_code
from easynet_sdk.errors import normalize_error_code, profile_error_details


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

    def test_normalize_error_code_canonicalizes_legacy_wire_aliases(self) -> None:
        cases = {
            "DAEMON_DOWN": ErrorCode.DAEMON_OFFLINE,
            "VERSION_INCOMPATIBLE": ErrorCode.VERSION_MISMATCH,
            "ABILITY_FAILED": ErrorCode.ADMISSION_DENIED,
            "NOT_FOUND": ErrorCode.ABILITY_NOT_FOUND,
            "PROTOCOL": ErrorCode.PROTOCOL_MISMATCH,
            "TRANSPORT": ErrorCode.ROUTE_UNAVAILABLE,
            "DAEMON_OFFLINE": ErrorCode.DAEMON_OFFLINE,
            "VERSION_MISMATCH": ErrorCode.VERSION_MISMATCH,
            "ADMISSION_DENIED": ErrorCode.ADMISSION_DENIED,
            "ABILITY_NOT_FOUND": ErrorCode.ABILITY_NOT_FOUND,
            "PROTOCOL_MISMATCH": ErrorCode.PROTOCOL_MISMATCH,
            "ROUTE_UNAVAILABLE": ErrorCode.ROUTE_UNAVAILABLE,
        }
        for raw, expected in cases.items():
            with self.subTest(raw=raw):
                self.assertEqual(normalize_error_code(raw), expected)

    def test_is_code_matches_canonicalized_legacy_requests(self) -> None:
        error = SDKError(
            code=ErrorCode.ROUTE_UNAVAILABLE,
            stage="transport",
            retry=RetryHint.SAFE,
            message="route down",
        )

        self.assertTrue(is_code(error, ErrorCode.TRANSPORT))
        self.assertTrue(is_code(error, ErrorCode.ROUTE_UNAVAILABLE))

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

    def test_profile_error_details_adds_stable_profile_refs(self) -> None:
        details = profile_error_details(
            "publication",
            details={"reason": "resource_ref_namespace_reserved"},
            operation="build_local_resource_ref",
        )

        self.assertEqual(details["profile"], "publication")
        self.assertEqual(details["source_ref"], "python_sdk.profile.publication")
        self.assertEqual(details["reason"], "resource_ref_namespace_reserved")
        self.assertEqual(details["operation"], "build_local_resource_ref")

    def test_profile_error_details_preserves_caller_refs(self) -> None:
        details = profile_error_details(
            "mission",
            source_ref="fixture.profile.source",
            details={"profile": "custom", "source_ref": "custom.source"},
            operation="run_file",
        )

        self.assertEqual(details["profile"], "custom")
        self.assertEqual(details["source_ref"], "custom.source")
        self.assertEqual(details["operation"], "run_file")


if __name__ == "__main__":
    unittest.main()
