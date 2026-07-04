import json
import unittest
from unittest.mock import patch

from easynet_sdk import (
    EasyRemoteInvocationRequest,
    ErrorCode,
    InvocationBuilder,
    InvocationDraft,
    RetryHint,
    SDKError,
    encode_easyremote_invocation,
    is_code,
)


class InvocationTests(unittest.TestCase):
    def test_easyremote_encoder_preserves_legacy_json_wire_shape(self) -> None:
        request = EasyRemoteInvocationRequest(
            caller_ura="easynet:///r/example/device/caller",
            callee_ura="easynet:///r/example/device/callee",
            ability="observe.health",
            subject_ura="easynet:///r/example/device/callee",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context={"form": "none"},
            args={"ping": True},
            has_json_args=True,
            content_type="application/json",
        )

        with _identity_projection_patches():
            wire = encode_easyremote_invocation(request)

        self.assertEqual(
            wire,
            {
                "caller_ura": "easynet:///r/example/device/caller",
                "callee_ura": "easynet:///r/example/device/callee",
                "descriptor_ref": (
                    "easynet:///r/example/ability/device.callee.observe.health@1.0.0"
                ),
                "subject_ura": "easynet:///r/example/device/callee",
                "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                "causal_context": {"form": "none"},
                "args": {"ping": True},
            },
        )
        self.assertNotIn("content_type", wire)

    def test_easyremote_encoder_preserves_binary_and_optional_extras(self) -> None:
        request = EasyRemoteInvocationRequest(
            caller_ura="easynet:///r/example/device/caller",
            callee_ura="easynet:///r/example/device/callee",
            ability="easynet:///r/example/ability/device.callee.observe.health@2.0.0",
            subject_ura="easynet:///r/example/device/callee",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context={"form": "none"},
            content_type="audio/pcm",
            arguments_base64="AAE=",
            metadata={"trace": "t1"},
            caller_signature={
                "algorithm": "ed25519",
                "signature_base64": "c2ln",
                "key_id_hint": "caller",
            },
            bidi_streams=({"stream_id": 1, "content_type": "text/pty"},),
        )

        with _identity_projection_patches():
            wire = encode_easyremote_invocation(request)

        self.assertEqual(
            wire["descriptor_ref"],
            "easynet:///r/example/ability/device.callee.observe.health@2.0.0",
        )
        self.assertEqual(wire["arguments_base64"], "AAE=")
        self.assertEqual(wire["content_type"], "audio/pcm")
        self.assertEqual(wire["metadata"], {"trace": "t1"})
        self.assertEqual(wire["caller_signature"]["algorithm"], "ed25519")
        self.assertEqual(
            wire["bidi_streams"],
            [{"stream_id": 1, "content_type": "text/pty"}],
        )

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


def _identity_projection_patches():
    def canonical(value: str) -> str:
        if "@" not in value:
            raise SDKError(
                code=ErrorCode.INVALID_ARGUMENT,
                stage="directory_identity",
                retry=RetryHint.NEVER,
                message="not a descriptor ref",
            )
        return value

    def owner_ability(owner_ura: str, ability_name: str) -> str:
        node = owner_ura.rsplit("/", 1)[-1]
        return f"easynet:///r/example/ability/device.{node}.{ability_name}"

    return patch.multiple(
        "easynet_sdk.invocation",
        _canonical_ability_descriptor_ref=canonical,
        _owner_ability_ura=owner_ability,
    )


if __name__ == "__main__":
    unittest.main()
