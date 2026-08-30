import unittest
from dataclasses import replace

from easynet_sdk import (
    ErrorCode,
    LocalRuntimeSignerProvider,
    ManagedSigner,
    ManagedSigningKey,
    ManagedSigningStatus,
    SDKError,
    Signer,
    SignerHandle,
)


CALLER_URA = "easynet:///r/acme/user/00000000-0000-4000-8000-000000000001"


class LocalRuntimeSignerProviderTests(unittest.TestCase):
    def test_resolves_active_key_service_signer(self) -> None:
        managed = _managed_signer("key-active")
        provider = LocalRuntimeSignerProvider(
            managed_signer_loader=lambda caller: managed,
        )

        resolved = provider.resolve(CALLER_URA)

        self.assertIs(resolved.provider, managed)
        self.assertEqual(resolved.handle.key_id, "key-active")
        self.assertEqual(resolved.handle.owner_ura, CALLER_URA)

    def test_matching_managed_signer_is_only_a_selection_pin(self) -> None:
        managed = _managed_signer("key-active")
        requested = managed.invocation_signer()
        provider = LocalRuntimeSignerProvider(
            managed_signer_loader=lambda caller: managed,
        )

        resolved = provider.resolve(CALLER_URA, requested)

        self.assertIsNot(resolved, requested)
        self.assertIs(resolved.provider, managed)
        self.assertEqual(resolved.handle, requested.handle)

    def test_rejects_rotated_or_foreign_managed_signer(self) -> None:
        active = _managed_signer("key-active")
        provider = LocalRuntimeSignerProvider(
            managed_signer_loader=lambda caller: active,
        )

        for requested in (
            _managed_signer("key-rotated").invocation_signer(),
            _managed_signer("key-active", owner_ura="easynet:///r/acme/user/bob")
            .invocation_signer(),
        ):
            with self.subTest(requested=requested.handle):
                with self.assertRaises(SDKError) as caught:
                    provider.resolve(CALLER_URA, requested)
                self.assertEqual(
                    caught.exception.code,
                    ErrorCode.CALLER_SIGNER_UNAVAILABLE,
                )
                self.assertEqual(caught.exception.stage, "runtime_signer")

    def test_rejects_non_key_service_signer_before_dispatch(self) -> None:
        active = _managed_signer("key-active")
        forged = Signer(
            handle=active.invocation_signer().handle,
            provider=_ForgedProvider(),
        )
        provider = LocalRuntimeSignerProvider(
            managed_signer_loader=lambda caller: active,
        )

        with self.assertRaises(SDKError) as caught:
            provider.resolve(CALLER_URA, forged)

        self.assertEqual(caught.exception.code, ErrorCode.CALLER_SIGNER_UNAVAILABLE)
        self.assertEqual(caught.exception.stage, "runtime_signer")

    def test_rejects_handle_that_forges_managed_provider_policy(self) -> None:
        active = _managed_signer("key-active")
        canonical = active.invocation_signer()
        forged = replace(
            canonical,
            handle=replace(
                canonical.handle,
                policy={
                    **canonical.handle.policy,
                    "policy_ref": "provider-key-inventory:sha256:forged",
                },
            ),
        )
        provider = LocalRuntimeSignerProvider(
            managed_signer_loader=lambda caller: active,
        )

        with self.assertRaises(SDKError) as caught:
            provider.resolve(CALLER_URA, forged)

        self.assertEqual(caught.exception.code, ErrorCode.CALLER_SIGNER_UNAVAILABLE)
        self.assertEqual(caught.exception.stage, "runtime_signer")

    def test_rejects_loader_that_bypasses_managed_key_custody(self) -> None:
        provider = LocalRuntimeSignerProvider(
            managed_signer_loader=lambda caller: _ForgedProvider(),  # type: ignore[arg-type,return-value]
        )

        with self.assertRaises(SDKError) as caught:
            provider.resolve(CALLER_URA)

        self.assertEqual(caught.exception.code, ErrorCode.CALLER_SIGNER_UNAVAILABLE)
        self.assertEqual(caught.exception.stage, "runtime_signer")


class _ForgedProvider:
    def sign(self, material: object, handle: SignerHandle) -> object:
        raise AssertionError("forged provider must never be called")


def _managed_signer(
    key_id: str,
    *,
    owner_ura: str = CALLER_URA,
) -> ManagedSigner:
    return ManagedSigner(
        key=ManagedSigningKey(
            key_id=key_id,
            purpose="user_signing.cli",
            public_key=bytes(range(32)),
            status=ManagedSigningStatus.ACTIVE,
            rotation_epoch=0,
            bound_subject_ura=owner_ura,
            signer_policy_ref=f"provider-key-inventory:sha256:{key_id}",
            rotated_from=None,
            created_unix_ms=1,
            expires_unix_ms=None,
            revoked_unix_ms=None,
        ),
        socket_path="/tmp/test-keyring.sock",
    )


if __name__ == "__main__":
    unittest.main()
