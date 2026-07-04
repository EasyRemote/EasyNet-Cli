import unittest

from easynet_sdk import (
    ErrorCode,
    InvocationSignature,
    PreparedInvocation,
    SDKError,
    Signer,
    SignerHandle,
    is_code,
)


PREPARED_FIXTURE = b"""{
  "prepared_id": "prepared-example-1",
  "tuple": {
    "caller_ura": "easynet:///r/example/agent/alice.sdk",
    "callee_ura": "easynet:///r/example/device/dev-a",
    "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
    "subject_ura": "easynet:///r/example/device/dev-a",
    "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
    "causal_context": {"form": "none"},
    "args": {},
    "content_type": "application/json",
    "metadata": {}
  },
  "signing_material": {
    "algorithm": "ed25519",
    "canonical_bytes_base64": "ZXhhbXBsZS1jYW5vbmljYWwtYnl0ZXM=",
    "args_digest_hex": "0000000000000000000000000000000000000000000000000000000000000000",
    "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
    "expires_at_unix_ms": 1783000000000
  },
  "submit_ready": false
}"""


class SigningTests(unittest.TestCase):
    def test_prepared_invocation_decodes_signing_material_fixture(self) -> None:
        prepared = PreparedInvocation.from_json(PREPARED_FIXTURE)

        self.assertFalse(prepared.submit_ready())
        self.assertEqual(prepared.prepared_id, "prepared-example-1")
        self.assertEqual(
            prepared.signing_material.descriptor_ref,
            prepared.descriptor_ref,
        )
        self.assertTrue(prepared.signing_material.canonical_bytes_base64)

    def test_prepared_invocation_rejects_missing_canonical_bytes(self) -> None:
        with self.assertRaises(SDKError) as caught:
            PreparedInvocation.from_json(
                b"""{
                    "prepared_id": "prepared-example-1",
                    "tuple": {
                        "caller_ura": "easynet:///r/example/agent/alice.sdk",
                        "callee_ura": "easynet:///r/example/device/dev-a",
                        "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
                        "subject_ura": "easynet:///r/example/device/dev-a",
                        "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                        "causal_context": {"form": "none"},
                        "args": {},
                        "content_type": "application/json"
                    },
                    "signing_material": {
                        "args_digest_hex": "00",
                        "expires_at_unix_ms": 1783000000000
                    }
                }"""
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_prepared_invocation_decodes_current_abi_shape(self) -> None:
        prepared = PreparedInvocation.from_json(
            b"""{
                "request_id": "req-1",
                "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
                "descriptor_hash_hex": "aa",
                "schema_hash_hex": "bb",
                "canonical_hash_hex": "cc",
                "expires_at_unix_ms": 1783000000000,
                "tuple": {
                    "caller_ura": "easynet:///r/example/agent/alice.sdk",
                    "callee_ura": "easynet:///r/example/device/dev-a",
                    "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
                    "subject_ura": "easynet:///r/example/device/dev-a",
                    "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                    "causal_context": {"form": "none"},
                    "args": {},
                    "content_type": "application/json"
                },
                "signing_material": {
                    "canonical_bytes_base64": "ZXhhbXBsZQ==",
                    "args_digest_hex": "00",
                    "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                    "signed_fields": ["caller_ura", "callee_ura"],
                    "signer_policy": {
                        "mode": "caller_signing",
                        "signer_id": "browser-key",
                        "policy_ref": "policy/local",
                        "expires_at_unix_ms": 1783000000000
                    },
                    "expires_at_unix_ms": 1783000000000
                }
            }"""
        )

        self.assertEqual(prepared.request_id, "req-1")
        self.assertIsNotNone(prepared.signing_material.signer_policy)
        assert prepared.signing_material.signer_policy is not None
        self.assertEqual(prepared.signing_material.signer_policy.signer_id, "browser-key")

    def test_prepared_invocation_rejects_submit_ready_payload(self) -> None:
        with self.assertRaises(SDKError) as caught:
            PreparedInvocation.from_json(
                b"""{
                    "prepared_id": "prepared-example-1",
                    "tuple": {
                        "caller_ura": "easynet:///r/example/agent/alice.sdk",
                        "callee_ura": "easynet:///r/example/device/dev-a",
                        "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
                        "subject_ura": "easynet:///r/example/device/dev-a",
                        "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                        "causal_context": {"form": "none"},
                        "args": {},
                        "content_type": "application/json"
                    },
                    "signing_material": {
                        "canonical_bytes_base64": "ZXhhbXBsZQ==",
                        "args_digest_hex": "00",
                        "expires_at_unix_ms": 1783000000000
                    },
                    "submit_ready": true
                }"""
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_prepared_invocation_signs_into_submit_ready_envelope(self) -> None:
        prepared = PreparedInvocation.from_json(PREPARED_FIXTURE)

        signed = prepared.sign_with_caller_signature(
            InvocationSignature(
                algorithm="ed25519",
                signature_base64="c2lnbmF0dXJl",
                key_id_hint="caller-key",
            )
        )

        self.assertTrue(signed.submit_ready())
        self.assertFalse(signed.prepared.submit_ready())
        self.assertEqual(signed.signer_id, "caller-key")

    def test_signer_provider_signs_with_daemon_authorized_handle(self) -> None:
        class MemorySignatureProvider:
            def sign(self, material, handle):
                self.material = material
                self.handle = handle
                return InvocationSignature(
                    algorithm="",
                    signature_base64="c2lnbmF0dXJl",
                )

        prepared = PreparedInvocation.from_json(PREPARED_FIXTURE)
        handle = signer_handle()
        provider = MemorySignatureProvider()
        signer = Signer(handle=handle, provider=provider)

        signed = signer.sign(prepared)

        self.assertTrue(signed.submit_ready())
        self.assertEqual(signed.signer_id, handle.signer_id)
        self.assertEqual(signed.signature.algorithm, handle.algorithm)
        self.assertEqual(signed.signature.key_id_hint, handle.signer_id)
        self.assertEqual(provider.material, prepared.signing_material)
        self.assertEqual(provider.handle, handle)

    def test_signer_rejects_policy_mismatch(self) -> None:
        prepared = PreparedInvocation.from_json(
            b"""{
                "prepared_id": "prepared-example-1",
                "tuple": {
                    "caller_ura": "easynet:///r/example/agent/alice.sdk",
                    "callee_ura": "easynet:///r/example/device/dev-a",
                    "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
                    "subject_ura": "easynet:///r/example/device/dev-a",
                    "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
                    "causal_context": {"form": "none"},
                    "args": {},
                    "content_type": "application/json"
                },
                "signing_material": {
                    "canonical_bytes_base64": "ZXhhbXBsZQ==",
                    "args_digest_hex": "00",
                    "expires_at_unix_ms": 1783000000000,
                    "signer_policy": {
                        "mode": "local_daemon_signing",
                        "signer_id": "other-signer"
                    }
                }
            }"""
        )
        signer = Signer.from_signature(
            signer_handle(),
            InvocationSignature(
                algorithm="ed25519",
                signature_base64="c2lnbmF0dXJl",
            ),
        )

        with self.assertRaises(SDKError) as caught:
            signer.sign(prepared)

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_signer_rejects_algorithm_mismatch(self) -> None:
        prepared = PreparedInvocation.from_json(PREPARED_FIXTURE)
        signer = Signer.from_signature(
            signer_handle(),
            InvocationSignature(
                algorithm="secp256k1",
                signature_base64="c2lnbmF0dXJl",
            ),
        )

        with self.assertRaises(SDKError) as caught:
            signer.sign(prepared)

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_prepared_invocation_rejects_empty_signature(self) -> None:
        prepared = PreparedInvocation.from_json(PREPARED_FIXTURE)

        with self.assertRaises(SDKError) as caught:
            prepared.sign_with_caller_signature(
                InvocationSignature(algorithm="ed25519", signature_base64="")
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))


def signer_handle() -> SignerHandle:
    return SignerHandle(
        profile="directory_identity",
        signer_id="signer-alice-key-1",
        owner_ura="easynet:///r/example/agent/alice.sdk",
        key_id="alice-key-1",
        algorithm="ed25519",
        policy={"mode": "local_daemon_signing", "usage": "invocation.sign"},
        metadata={"source": "daemon_keyring"},
    )


if __name__ == "__main__":
    unittest.main()
