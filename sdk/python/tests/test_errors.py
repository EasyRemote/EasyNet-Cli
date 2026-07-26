import unittest

from easynet_sdk import ErrorClass, ErrorCode, RetryHint, SDKError, is_code
from easynet_sdk.errors import (
    canonical_failure_code,
    error_class_for_code,
    normalize_error_code,
    profile_error_details,
    profile_source_ref,
)


class ErrorTests(unittest.TestCase):
    def test_from_json_decodes_fixture_shape(self) -> None:
        error = SDKError.from_json(
            b"""{
                "code": "INVALID_ARGUMENT",
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
                "receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/opaque/receipt",
                "details": {"abi_symbol": "ERR_TIMEOUT"}
            }"""
        )

        self.assertIsNotNone(error)
        assert error is not None
        self.assertEqual(error.code, ErrorCode.TIMEOUT)
        self.assertTrue(error.retryable)
        self.assertEqual(error.invocation_id, "inv-1")
        self.assertEqual(error.receipt_ura, "easynet:///r/example/resource/agent.alice.sdk/invocation/opaque/receipt")
        self.assertEqual(error.details["abi_symbol"], "ERR_TIMEOUT")
        self.assertEqual(error.error_class, ErrorClass.TIMEOUT)

    def test_from_json_canonicalizes_caller_signer_custody_detail(self) -> None:
        error = SDKError.from_json(
            b"""{
                "code": "CALLER_SIGNER_UNAVAILABLE",
                "stage": "caller_identity",
                "message": "easynet_runtime_resolve_descriptor_ref: remote invocation requires a caller signer for `easynet:///r/localhost/user/alice`; load or provision that identity in the local key service: self-identity: keyring rejected request: kind=not_found, msg=keyring entry not found: easynet:///r/localhost/user/alice",
                "retry": "never",
                "source": "c_abi",
                "invocation_id": "inv-1",
                "receipt_ura": null,
                "details": {"abi_symbol": "ERR_PERMISSION_DENIED"}
            }"""
        )

        self.assertIsNotNone(error)
        assert error is not None
        self.assertEqual(error.code, ErrorCode.CALLER_SIGNER_UNAVAILABLE)
        self.assertEqual(
            error.message,
            "CALLER_SIGNER_UNAVAILABLE: remote invocation requires a caller signer for `easynet:///r/localhost/user/alice`; load or provision that identity in the local key service",
        )
        for leaked in (
            "keyring entry not found",
            "keyring rejected request",
            "self-identity:",
        ):
            self.assertNotIn(leaked, error.message)
        self.assertEqual(error.stage, "caller_identity")
        self.assertEqual(error.source, "c_abi")
        self.assertEqual(error.invocation_id, "inv-1")
        self.assertEqual(error.details["abi_symbol"], "ERR_PERMISSION_DENIED")

    def test_normalize_error_code_accepts_only_canonical_schema_values(self) -> None:
        cases = {
            "VERSION_INCOMPATIBLE": ErrorCode.VERSION_INCOMPATIBLE,
            "ABILITY_FAILED": ErrorCode.ABILITY_FAILED,
            "NOT_FOUND": ErrorCode.NOT_FOUND,
            "PROTOCOL": ErrorCode.PROTOCOL,
            "TRANSPORT": ErrorCode.TRANSPORT,
            "RUNTIME_OFFLINE": ErrorCode.RUNTIME_OFFLINE,
            "VERSION_MISMATCH": ErrorCode.VERSION_MISMATCH,
            "ADMISSION_DENIED": ErrorCode.ADMISSION_DENIED,
            "HTTP_AUTH_DENIED": ErrorCode.HTTP_AUTH_DENIED,
            "SIGNATURE_DENIED": ErrorCode.SIGNATURE_DENIED,
            "POLICY_DENIED": ErrorCode.POLICY_DENIED,
            "AUTHORITY_DENIED": ErrorCode.AUTHORITY_DENIED,
            "ABILITY_NOT_FOUND": ErrorCode.ABILITY_NOT_FOUND,
            "PROTOCOL_MISMATCH": ErrorCode.PROTOCOL_MISMATCH,
            "ROUTE_UNAVAILABLE": ErrorCode.ROUTE_UNAVAILABLE,
            "EXECUTION_FAILED": ErrorCode.EXECUTION_FAILED,
        }
        for raw, expected in cases.items():
            with self.subTest(raw=raw):
                self.assertEqual(normalize_error_code(raw), expected)

    def test_from_json_rejects_legacy_error_code_aliases(self) -> None:
        for code in ("InvalidArgument", "DaemonDown", "DAEMON_DOWN", "DAEMON_OFFLINE", "VersionIncompatible"):
            with self.subTest(code=code):
                with self.assertRaises(SDKError) as caught:
                    SDKError.from_json(
                        f"""{{
                            "code": "{code}",
                            "stage": "transport",
                            "message": "legacy code",
                            "retry": "never",
                            "details": {{}}
                        }}"""
                    )
                self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_canonical_failure_code_preserves_domain_codes_and_rejects_aliases(
        self,
    ) -> None:
        cases = {
            "": ErrorCode.PROTOCOL_MISMATCH,
            "   ": ErrorCode.PROTOCOL_MISMATCH,
            "TRANSPORT": ErrorCode.TRANSPORT,
            " AXON_MEMBERSHIP_REQUIRED ": "AXON_MEMBERSHIP_REQUIRED",
            "TARGET_NOT_IN_PRESENCE_REGISTRY": "TARGET_NOT_IN_PRESENCE_REGISTRY",
            "InvalidArgument": ErrorCode.PROTOCOL_MISMATCH,
            "DAEMON_DOWN": ErrorCode.PROTOCOL_MISMATCH,
            "DAEMON_OFFLINE": ErrorCode.PROTOCOL_MISMATCH,
        }
        for raw, expected in cases.items():
            with self.subTest(raw=raw):
                self.assertEqual(canonical_failure_code(raw), expected)

    def test_extension_failure_codes_project_to_generic_class(self) -> None:
        error = SDKError(
            code="AXON_MEMBERSHIP_REQUIRED",
            stage="transport",
            retry=RetryHint.UNKNOWN,
            message="membership required",
        )

        self.assertEqual(error.error_class, ErrorClass.GENERIC)
        self.assertFalse(is_code(error, ErrorCode.PROTOCOL_MISMATCH))

    def test_error_class_for_code_projects_stable_classes(self) -> None:
        cases = {
            ErrorCode.INVALID_ARGUMENT: ErrorClass.VALIDATION,
            ErrorCode.INVALID_HANDLE: ErrorClass.HANDLE,
            ErrorCode.NOT_INITIALIZED: ErrorClass.LIFECYCLE,
            ErrorCode.RUNTIME_OFFLINE: ErrorClass.AVAILABILITY,
            ErrorCode.TRANSPORT: ErrorClass.AVAILABILITY,
            ErrorCode.PERMISSION_DENIED: ErrorClass.PERMISSION,
            ErrorCode.HTTP_AUTH_DENIED: ErrorClass.PERMISSION,
            ErrorCode.ADMISSION_DENIED: ErrorClass.ADMISSION,
            ErrorCode.SIGNATURE_DENIED: ErrorClass.ADMISSION,
            ErrorCode.POLICY_DENIED: ErrorClass.ADMISSION,
            ErrorCode.AUTHORITY_DENIED: ErrorClass.ADMISSION,
            ErrorCode.EXECUTION_FAILED: ErrorClass.ADMISSION,
            ErrorCode.ABILITY_FAILED: ErrorClass.ADMISSION,
            ErrorCode.ABILITY_NOT_FOUND: ErrorClass.ROUTING,
            ErrorCode.NOT_FOUND: ErrorClass.ROUTING,
            ErrorCode.TIMEOUT: ErrorClass.TIMEOUT,
            ErrorCode.CANCELLED: ErrorClass.CANCELLATION,
            ErrorCode.PROTOCOL_MISMATCH: ErrorClass.PROTOCOL,
            ErrorCode.PROTOCOL: ErrorClass.PROTOCOL,
            ErrorCode.VERSION_MISMATCH: ErrorClass.VERSION,
            ErrorCode.VERSION_INCOMPATIBLE: ErrorClass.VERSION,
            ErrorCode.CONTROL_ONLY: ErrorClass.CONTROL,
            ErrorCode.NOT_IMPLEMENTED: ErrorClass.UNSUPPORTED,
            ErrorCode.GENERIC: ErrorClass.GENERIC,
        }
        for code, expected in cases.items():
            with self.subTest(code=code):
                self.assertEqual(error_class_for_code(code), expected)

    def test_is_code_matches_exact_canonical_requests(self) -> None:
        error = SDKError(
            code=ErrorCode.ROUTE_UNAVAILABLE,
            stage="transport",
            retry=RetryHint.SAFE,
            message="route down",
        )

        self.assertFalse(is_code(error, ErrorCode.TRANSPORT))
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
            "directory",
            details={"reason": "canonical_projection_rejected"},
            operation="resolve_ura",
        )

        self.assertEqual(details["profile"], "directory")
        self.assertEqual(details["source_ref"], "python_sdk.profile.directory")
        self.assertEqual(details["reason"], "canonical_projection_rejected")
        self.assertEqual(details["operation"], "resolve_ura")

    def test_profile_error_details_preserves_caller_refs(self) -> None:
        details = profile_error_details(
            "authority",
            source_ref="fixture.profile.source",
            details={"profile": "custom", "source_ref": "custom.source"},
            operation="mint_session_authority",
        )

        self.assertEqual(details["profile"], "custom")
        self.assertEqual(details["source_ref"], "custom.source")
        self.assertEqual(details["operation"], "mint_session_authority")

    def test_sdk_error_profile_and_source_ref_accessors(self) -> None:
        error = SDKError(
            code=ErrorCode.INVALID_ARGUMENT,
            stage="directory",
            retry=RetryHint.NEVER,
            message="invalid directory projection",
            details=profile_error_details(
                "directory",
                details={"reason": "canonical_projection_rejected"},
            ),
        )

        self.assertEqual(error.profile, "directory")
        self.assertEqual(error.source_ref, "python_sdk.profile.directory")
        self.assertEqual(error.error_class, ErrorClass.VALIDATION)
        self.assertEqual(
            profile_source_ref(" directory "), "python_sdk.profile.directory"
        )


if __name__ == "__main__":
    unittest.main()
