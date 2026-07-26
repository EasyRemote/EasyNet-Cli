import base64
import json
import unittest

from easynet_sdk import (
    ErrorCode,
    InvocationBuilder,
    InvocationDraft,
    InvocationSignature,
    SDKError,
    is_code,
    new_invocation_nonce_base64,
)


class InvocationTests(unittest.TestCase):
    def test_new_invocation_nonce_base64_returns_sixteen_bytes(self) -> None:
        nonce = new_invocation_nonce_base64()
        self.assertEqual(len(base64.b64decode(nonce)), 16)

    def test_builder_builds_complete_tuple(self) -> None:
        draft = (
            InvocationBuilder()
            .with_caller_ura("easynet:///r/example/agent/alice.sdk")
            .with_callee_ura("easynet:///r/example/device/dev-a")
            .with_descriptor_ref(
                "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
            )
            .with_subject_ura("easynet:///r/example/device/dev-a")
            .with_nonce_base64("AQIDBAUGBwgJCgsMDQ4PEA==")
            .with_causal_context({"form": "none"})
            .with_json_args({})
            .with_content_type("application/json")
            .with_metadata({})
            .build()
        )

        payload = draft.to_json_dict()

        self.assertIn("args", payload)
        self.assertNotIn("arguments_base64", payload)
        self.assertEqual(payload["caller_ura"], "easynet:///r/example/agent/alice.sdk")

    def test_builder_inspect_does_not_consume_and_build_consumes(self) -> None:
        builder = (
            InvocationBuilder()
            .with_caller_ura("easynet:///r/example/agent/alice.sdk")
            .with_callee_ura("easynet:///r/example/device/dev-a")
            .with_descriptor_ref(
                "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
            )
            .with_subject_ura("easynet:///r/example/device/dev-a")
            .with_nonce_base64("AQIDBAUGBwgJCgsMDQ4PEA==")
            .with_causal_context({"form": "none"})
            .with_json_args({})
            .with_content_type("application/json")
        )

        builder.inspect()
        builder.inspect()
        builder.build()

        with self.assertRaises(SDKError) as inspect_caught:
            builder.inspect()
        self.assertTrue(is_code(inspect_caught.exception, ErrorCode.INVALID_HANDLE))

        with self.assertRaises(SDKError) as build_caught:
            builder.build()
        self.assertTrue(is_code(build_caught.exception, ErrorCode.INVALID_HANDLE))

    def test_draft_from_json_decodes_fixture_shape(self) -> None:
        draft = InvocationDraft.from_json(
            b"""{
                "caller_ura": "easynet:///r/example/agent/alice.sdk",
                "callee_ura": "easynet:///r/example/device/dev-a",
                "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
                "subject_ura": "easynet:///r/example/device/dev-a",
                "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                "causal_context": {"form": "none"},
                "args": {},
                "content_type": "application/json",
                "metadata": {}
            }"""
        )

        self.assertEqual(draft.caller_ura, "easynet:///r/example/agent/alice.sdk")
        self.assertIn("args", json.loads(draft.to_json()))

    def test_draft_rejects_signer_public_key_without_key_hint(self) -> None:
        with self.assertRaises(SDKError) as caught:
            (
                complete_builder()
                .with_caller_signature(
                    InvocationSignature(
                        algorithm="ed25519",
                        signature_base64=(
                            "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBw=="
                        ),
                        signer_public_key_base64=(
                            "o5TNp0VYb4h93vG8tNTXOh9gSePT3OYkGq1hlOYrmsM="
                        ),
                    )
                )
                .build()
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertIn("caller_signature.key_id_hint", str(caught.exception))

    def test_builder_rejects_missing_tuple_field(self) -> None:
        with self.assertRaises(SDKError) as caught:
            (
                InvocationBuilder()
                .with_callee_ura("easynet:///r/example/device/dev-a")
                .with_descriptor_ref(
                    "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
                )
                .with_subject_ura("easynet:///r/example/device/dev-a")
                .with_nonce_base64("AQIDBAUGBwgJCgsMDQ4PEA==")
                .with_causal_context({"form": "none"})
                .with_json_args({})
                .with_content_type("application/json")
                .build()
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_builder_rejects_all_zero_principals(self) -> None:
        placeholder = (
            "easynet:///r/example/resource/"
            "user.00000000-0000-0000-0000-000000000000/runtime-state/read"
        )
        cases = {
            "caller": lambda: complete_builder().with_caller_ura(placeholder),
            "callee": lambda: complete_builder().with_callee_ura(placeholder),
            "subject": lambda: complete_builder().with_subject_ura(placeholder),
        }
        for name, build in cases.items():
            with self.subTest(name=name):
                with self.assertRaises(SDKError) as caught:
                    build().build()
                self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
                self.assertIn("must not be all-zero", str(caught.exception))

    def test_builder_rejects_dual_argument_carriers(self) -> None:
        with self.assertRaises(SDKError) as caught:
            (
                InvocationBuilder()
                .with_caller_ura("easynet:///r/example/agent/alice.sdk")
                .with_callee_ura("easynet:///r/example/device/dev-a")
                .with_descriptor_ref(
                    "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
                )
                .with_subject_ura("easynet:///r/example/device/dev-a")
                .with_nonce_base64("AQIDBAUGBwgJCgsMDQ4PEA==")
                .with_causal_context({"form": "none"})
                .with_json_args({})
                .with_arguments_base64("e30=")
                .with_content_type("application/json")
                .build()
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_builder_rejects_malformed_nonce(self) -> None:
        for nonce in ("not base64", "AQIDBA=="):
            with self.subTest(nonce=nonce):
                with self.assertRaises(SDKError) as caught:
                    (
                        InvocationBuilder()
                        .with_caller_ura("easynet:///r/example/agent/alice.sdk")
                        .with_callee_ura("easynet:///r/example/device/dev-a")
                        .with_descriptor_ref(
                            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
                        )
                        .with_subject_ura("easynet:///r/example/device/dev-a")
                        .with_nonce_base64(nonce)
                        .with_causal_context({"form": "none"})
                        .with_json_args({})
                        .with_content_type("application/json")
                        .build()
                    )

                self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_builder_rejects_malformed_raw_payload(self) -> None:
        with self.assertRaises(SDKError) as caught:
            (
                InvocationBuilder()
                .with_caller_ura("easynet:///r/example/agent/alice.sdk")
                .with_callee_ura("easynet:///r/example/device/dev-a")
                .with_descriptor_ref(
                    "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
                )
                .with_subject_ura("easynet:///r/example/device/dev-a")
                .with_nonce_base64("AQIDBAUGBwgJCgsMDQ4PEA==")
                .with_causal_context({"form": "none"})
                .with_arguments_base64("not base64")
                .with_content_type("application/octet-stream")
                .build()
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_draft_from_json_rejects_unknown_field(self) -> None:
        with self.assertRaises(SDKError) as caught:
            InvocationDraft.from_json(
                b"""{
                    "caller_ura": "easynet:///r/example/agent/alice.sdk",
                    "callee_ura": "easynet:///r/example/device/dev-a",
                    "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
                    "subject_ura": "easynet:///r/example/device/dev-a",
                    "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                    "causal_context": {"form": "none"},
                    "args": {},
                    "content_type": "application/json",
                    "unexpected": true
                }"""
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))


def complete_builder() -> InvocationBuilder:
    return (
        InvocationBuilder()
        .with_caller_ura("easynet:///r/example/agent/alice.sdk")
        .with_callee_ura("easynet:///r/example/device/dev-a")
        .with_descriptor_ref(
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
        )
        .with_subject_ura("easynet:///r/example/device/dev-a")
        .with_nonce_base64("AQIDBAUGBwgJCgsMDQ4PEA==")
        .with_causal_context({"form": "none"})
        .with_json_args({})
        .with_content_type("application/json")
    )


if __name__ == "__main__":
    unittest.main()
