import base64
import json
import unittest

from easynet_sdk import (
    ErrorCode,
    InvocationBuilder,
    InvocationDraft,
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

if __name__ == "__main__":
    unittest.main()
