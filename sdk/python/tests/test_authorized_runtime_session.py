import base64
import json
import unittest
from dataclasses import replace

from easynet_sdk import (
    AbilityRef,
    ActingPrincipalRef,
    AuthorityArtifact,
    AuthorizedRuntimeSession,
    CallerIdentityRef,
    DescriptorResolution,
    DescriptorResolutionState,
    ErrorCode,
    InvocationIntent,
    PrepareOptions,
    PreparedInvocation,
    ReceiptFilter,
    ReceiptListRequest,
    PrincipalRef,
    RuntimeCallContext,
    RuntimeClient,
    RuntimeClientDescriptorProvider,
    RuntimeTargetRef,
    RuntimeClientSessionRuntimeProvider,
    RetryHint,
    SDKError,
    SessionAuthority,
    SigningMaterial,
    StaticCallerIdentityProvider,
    SubjectRef,
    is_code,
    runtime_state_read_subject_ura,
)
from easynet_sdk.authorized_runtime_session import _descriptor_resolution_from_error
from easynet_sdk._session_authority_subjects import is_runtime_state_read_subject_ura


class AuthorizedRuntimeSessionTests(unittest.TestCase):
    def test_rejects_authority_subject_mismatch_before_dispatch(self) -> None:
        fixture = _SessionFixture()
        fixture.authorization.authority = _session_authority(
            {
                "session_owner_user_id": "bob",
                "subject_ura": "easynet:///r/example/resource/user.bob/session/session-1",
            }
        )

        with self.assertRaises(SDKError) as caught:
            fixture.session.invoke.submit(_intent(), PrepareOptions())

        self.assertTrue(is_code(caught.exception, ErrorCode.AUTHORITY_SUBJECT_MISMATCH))
        self.assertEqual(fixture.runtime.prepare_calls, 0)
        self.assertEqual(fixture.runtime.submit_calls, 0)

    def test_rejects_path_substring_owner_subject_before_dispatch(self) -> None:
        fixture = _SessionFixture()
        intent = replace(
            _intent(),
            subject=SubjectRef(
                "easynet:///r/example/resource/device.dev-a/archive/resource/user.alice/session/session-1",
                "fixture",
            ),
        )

        with self.assertRaises(SDKError) as caught:
            fixture.session.invoke.submit(intent, PrepareOptions())

        self.assertTrue(is_code(caught.exception, ErrorCode.AUTHORITY_SUBJECT_MISMATCH))
        self.assertEqual(fixture.runtime.prepare_calls, 0)
        self.assertEqual(fixture.runtime.submit_calls, 0)

    def test_rejects_retired_invocation_history_subject_exact_authority_before_dispatch(
        self,
    ) -> None:
        fixture = _SessionFixture()
        retired_subject = (
            "easynet:///r/example/resource/user.alice/session/invocation_history"
        )
        fixture.authorization.authority = SessionAuthority(
            issuer_ura="easynet:///r/example/agent/backend",
            session_id="invocation_history",
            session_owner_user_id="alice",
            session_owner_ura="easynet:///r/example/user/alice",
            creator_principal_id="easynet:///r/example/agent/backend",
            creator_principal_ura="easynet:///r/example/agent/backend",
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura=retired_subject,
            audience="easynet:///r/example/device/dev-a",
            scopes=("observe.health",),
            allowed_actions=("invoke",),
            allowed_followup_abilities=("observe.health",),
            issued_at_ms=1000,
            expires_at_ms=2000,
            signature=b"signature",
        )
        intent = replace(_intent(), subject=SubjectRef(retired_subject, "fixture"))

        with self.assertRaises(SDKError) as caught:
            fixture.session.invoke.submit(intent, PrepareOptions())

        self.assertTrue(is_code(caught.exception, ErrorCode.AUTHORITY_SUBJECT_MISMATCH))
        self.assertEqual(fixture.runtime.prepare_calls, 0)
        self.assertEqual(fixture.runtime.submit_calls, 0)

    def test_rejects_missing_caller_identity_before_descriptor(self) -> None:
        fixture = _SessionFixture(identity=StaticCallerIdentityProvider(CallerIdentityRef(PrincipalRef(""))))
        intent = _intent(caller="")

        with self.assertRaises(SDKError) as caught:
            fixture.session.abilities.resolve(intent)

        self.assertTrue(is_code(caught.exception, ErrorCode.CALLER_IDENTITY_UNAVAILABLE))
        self.assertEqual(fixture.descriptor.calls, 0)

    def test_rejects_missing_caller_signer_before_submit(self) -> None:
        fixture = _SessionFixture()
        fixture.signer.error = SDKError(
            code=ErrorCode.CALLER_SIGNER_UNAVAILABLE,
            stage="sign",
            retry=RetryHint.NEVER,
            retryable=False,
            message="no caller signer",
        )

        with self.assertRaises(SDKError) as caught:
            fixture.session.invoke.submit(_intent(), PrepareOptions())

        self.assertTrue(is_code(caught.exception, ErrorCode.CALLER_SIGNER_UNAVAILABLE))
        self.assertEqual(fixture.runtime.prepare_calls, 1)
        self.assertEqual(fixture.runtime.submit_calls, 0)

    def test_descriptor_resolution_requires_descriptor_vocabulary(self) -> None:
        canonical = _descriptor_resolution_from_error(
            SDKError(
                code=ErrorCode.DESCRIPTOR_NOT_FOUND,
                stage="descriptor",
                retry=RetryHint.NEVER,
                retryable=False,
                message="descriptor missing",
            )
        )
        self.assertEqual(canonical.state, DescriptorResolutionState.NOT_FOUND)

        for legacy_code in (ErrorCode.ABILITY_NOT_FOUND, ErrorCode.NOT_FOUND):
            with self.subTest(code=legacy_code):
                resolution = _descriptor_resolution_from_error(
                    SDKError(
                        code=legacy_code,
                        stage="descriptor",
                        retry=RetryHint.NEVER,
                        retryable=False,
                        message="legacy provider not found",
                    )
                )
                self.assertEqual(
                    resolution.state,
                    DescriptorResolutionState.UNAVAILABLE,
                )

    def test_descriptor_resolution_requires_typed_owner_offline(self) -> None:
        typed = _descriptor_resolution_from_error(
            SDKError(
                code=ErrorCode.DESCRIPTOR_OWNER_OFFLINE,
                stage="descriptor",
                retry=RetryHint.NEVER,
                retryable=False,
                message="owner is not online",
            )
        )
        self.assertEqual(typed.state, DescriptorResolutionState.OWNER_OFFLINE)

        generic = _descriptor_resolution_from_error(
            SDKError(
                code=ErrorCode.PROVIDER_UNAVAILABLE,
                stage="descriptor",
                retry=RetryHint.NEVER,
                retryable=False,
                message="owner is offline",
            )
        )
        self.assertEqual(generic.state, DescriptorResolutionState.UNAVAILABLE)

    def test_history_rejects_authority_subject_mismatch_before_receipt_provider(self) -> None:
        fixture = _SessionFixture()
        request = ReceiptListRequest(
            call=RuntimeCallContext(
                caller_ura="easynet:///r/example/agent/backend",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura=runtime_state_read_subject_ura("example", "alice"),
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                authority=_session_authority(
                    {
                        "session_owner_user_id": "bob",
                        "subject_ura": "easynet:///r/example/resource/user.bob/session/session-1",
                    }
                ),
            ),
            limit=10,
        )

        with self.assertRaises(SDKError) as caught:
            fixture.session.history.list(request)

        self.assertTrue(is_code(caught.exception, ErrorCode.AUTHORITY_SUBJECT_MISMATCH))
        self.assertEqual(fixture.receipts.list_calls, 0)

    def test_history_rejects_all_zero_subject_before_receipt_provider(self) -> None:
        fixture = _SessionFixture()
        request = ReceiptListRequest(
            call=RuntimeCallContext(
                caller_ura="easynet:///r/example/agent/backend",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura="easynet:///r/example/resource/user.00000000-0000-0000-0000-000000000000/session/invocation_history",
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                authority=_session_authority(),
            ),
            limit=10,
        )

        with self.assertRaisesRegex(SDKError, "subject_ura must not be all-zero"):
            fixture.session.history.list(request)

        self.assertEqual(fixture.receipts.list_calls, 0)

    def test_history_rejects_retired_session_subject_before_receipt_provider(
        self,
    ) -> None:
        fixture = _SessionFixture()
        request = ReceiptListRequest(
            call=RuntimeCallContext(
                caller_ura="easynet:///r/example/agent/backend",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura="easynet:///r/example/resource/user.alice/session/invocation_history",
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                authority=_session_authority(),
            ),
            limit=10,
        )

        with self.assertRaisesRegex(SDKError, "runtime-state read subject") as caught:
            fixture.session.history.list(request)

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_INVOCATION))
        self.assertEqual(fixture.receipts.list_calls, 0)

    def test_history_allows_user_owned_resource_subject_before_receipt_provider(self) -> None:
        fixture = _SessionFixture()
        request = ReceiptListRequest(
            call=RuntimeCallContext(
                caller_ura="easynet:///r/example/agent/backend",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura=runtime_state_read_subject_ura("example", "alice"),
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                authority=_session_authority(),
            ),
            limit=10,
        )

        fixture.session.history.list(request)

        self.assertEqual(fixture.receipts.list_calls, 1)

    def test_history_uses_receipt_provider_authority_scope(self) -> None:
        fixture = _SessionFixture()
        fixture.receipts.history_list_scope = "receipt.catalog.list"
        request = ReceiptListRequest(
            call=RuntimeCallContext(
                caller_ura="easynet:///r/example/agent/backend",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura=runtime_state_read_subject_ura("example", "alice"),
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                authority=_session_authority(
                    {
                        "scopes": ["receipt.catalog.list"],
                        "allowed_followup_abilities": ["receipt.catalog.list"],
                    }
                ),
            ),
            limit=10,
        )

        fixture.session.history.list(request)

        self.assertEqual(fixture.receipts.list_calls, 1)

    def test_history_rejects_provider_without_authority_scope(self) -> None:
        fixture = _SessionFixture(receipts=_ReceiptProviderWithoutScope())
        request = ReceiptListRequest(
            call=RuntimeCallContext(
                caller_ura="easynet:///r/example/agent/backend",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura=runtime_state_read_subject_ura("example", "alice"),
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                authority=_session_authority(),
            ),
            limit=10,
        )

        with self.assertRaisesRegex(
            SDKError, "receipt provider does not expose receipt history authority scope"
        ) as caught:
            fixture.session.history.list(request)

        self.assertTrue(is_code(caught.exception, ErrorCode.PROVIDER_UNAVAILABLE))

    def test_runtime_state_read_subject_ura_builds_user_owned_resource_subject(self) -> None:
        self.assertEqual(
            runtime_state_read_subject_ura("example", "alice"),
            "easynet:///r/example/resource/user.alice/runtime-state/read",
        )

    def test_runtime_state_read_subject_ura_rejects_all_zero_user_before_device_fallback(
        self,
    ) -> None:
        with self.assertRaisesRegex(SDKError, "user_id must not be all-zero"):
            runtime_state_read_subject_ura(
                "example", "00000000-0000-0000-0000-000000000000"
            )

    def test_runtime_state_read_subject_predicate_rejects_all_zero_owner(
        self,
    ) -> None:
        self.assertFalse(
            is_runtime_state_read_subject_ura(
                "easynet:///r/example/resource/user.00000000-0000-0000-0000-000000000000/runtime-state/read"
            )
        )

    def test_history_rejects_path_substring_owner_subject_before_receipt_provider(self) -> None:
        fixture = _SessionFixture()
        request = ReceiptListRequest(
            call=RuntimeCallContext(
                caller_ura="easynet:///r/example/agent/backend",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura="easynet:///r/example/resource/device.dev-a/archive/resource/user.alice/session/session-1",
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                authority=_session_authority(),
            ),
            limit=10,
        )

        with self.assertRaises(SDKError) as caught:
            fixture.session.history.list(request)

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_INVOCATION))
        self.assertEqual(fixture.receipts.list_calls, 0)

    def test_history_allows_session_authority_with_exact_device_subject_filter(self) -> None:
        fixture = _SessionFixture()
        request = ReceiptListRequest(
            call=RuntimeCallContext(
                caller_ura="easynet:///r/example/agent/backend",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura=runtime_state_read_subject_ura("example", "alice"),
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                authority=_session_authority(),
            ),
            filter=ReceiptFilter(
                subject_uras=("easynet:///r/example/device/dev-a",),
            ),
            limit=10,
        )

        fixture.session.history.list(request)

        self.assertEqual(fixture.receipts.list_calls, 1)

    def test_runtime_client_provider_opens_signed_stream_and_bidi(self) -> None:
        from test_runtime import MemoryRuntimeTransport, signed_fixture

        transport = MemoryRuntimeTransport()
        provider = RuntimeClientSessionRuntimeProvider(RuntimeClient(transport))
        signed = signed_fixture()

        stream = provider.open_stream(signed)
        self.assertEqual(stream.stream_id, "stream-1")
        assert transport.seen_draft is not None
        self.assertEqual(transport.seen_draft["signer_id"], "caller-key")

        bidi = provider.open_bidi(signed, ())
        self.assertEqual(bidi.session_id, "bidi-1")
        assert transport.seen_draft is not None
        self.assertEqual(
            transport.seen_draft["signature"]["signature_base64"],
            "c2lnbmF0dXJl",
        )

    def test_runtime_client_providers_reject_missing_client_at_construction(self) -> None:
        with self.assertRaises(SDKError) as runtime_error:
            RuntimeClientSessionRuntimeProvider(None)  # type: ignore[arg-type]
        self.assertTrue(is_code(runtime_error.exception, ErrorCode.PROVIDER_UNAVAILABLE))

        with self.assertRaises(SDKError) as descriptor_error:
            RuntimeClientDescriptorProvider(None)  # type: ignore[arg-type]
        self.assertTrue(is_code(descriptor_error.exception, ErrorCode.PROVIDER_UNAVAILABLE))


class _SessionFixture:
    def __init__(
        self, identity: object | None = None, receipts: object | None = None
    ) -> None:
        self.runtime = _RuntimeProvider()
        self.descriptor = _DescriptorProvider()
        self.authorization = _AuthorizationProvider()
        self.signer = _SignerProvider()
        self.identity = identity or StaticCallerIdentityProvider(
            CallerIdentityRef(PrincipalRef("easynet:///r/example/agent/backend"))
        )
        self.receipts = receipts or _ReceiptProvider()
        self.session = AuthorizedRuntimeSession(
            runtime=self.runtime,
            descriptor=self.descriptor,
            authorization=self.authorization,
            signer=self.signer,
            receipts=self.receipts,
            identity=self.identity,
            clock=_Clock(),
        )


class _RuntimeProvider:
    def __init__(self) -> None:
        self.prepare_calls = 0
        self.submit_calls = 0

    def prepare_for_signing(
        self, draft, options: PrepareOptions
    ) -> tuple[PreparedInvocation, SigningMaterial]:
        self.prepare_calls += 1
        raw = {
            "prepared_id": "prepared-1",
            "descriptor_ref": draft.descriptor_ref,
            "expires_at_unix_ms": 3000,
            "tuple": json.loads(draft.to_json()),
            "signing_material": {
                "canonical_bytes_base64": base64.b64encode(b"canonical").decode("ascii"),
                "args_digest_hex": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "descriptor_ref": draft.descriptor_ref,
                "expires_at_unix_ms": 3000,
                "signed_fields": ["caller_ura"],
            },
        }
        prepared = PreparedInvocation.from_json(json.dumps(raw))
        return prepared, prepared.signing_material

    def submit_signed(self, signed):
        self.submit_calls += 1
        return None

    def await_terminal(self, handle):
        return None

    def open_stream(self, signed):
        return None

    def open_bidi(self, signed, streams):
        return None

    def cancel(self, handle, reason=""):
        return None

    def events(self, handle):
        return None

    def diagnostics(self):
        return {}


class _DescriptorProvider:
    def __init__(self) -> None:
        self.calls = 0

    def resolve_descriptor(self, request) -> DescriptorResolution:
        self.calls += 1
        return DescriptorResolution(
            state=DescriptorResolutionState.RESOLVED,
            descriptor_ref="easynet:///r/example/ability/invocation.history.list@1.0.0",
            descriptor_fingerprint="descriptor-fingerprint",
            owner_principal=PrincipalRef("easynet:///r/example/user/alice"),
        )


class _AuthorizationProvider:
    def __init__(self) -> None:
        self.authority = _session_authority()

    def authorize_invocation(self, prepared) -> AuthorityArtifact:
        return AuthorityArtifact(
            authority=self.authority,
            fingerprint="authority-fingerprint",
            subject=SubjectRef(self.authority.subject_ura),
            owner=PrincipalRef("easynet:///r/example/user/alice"),
        )


class _SignerProvider:
    def __init__(self) -> None:
        self.error: SDKError | None = None

    def caller_signer(self, authorized, material):
        if self.error is not None:
            raise self.error
        raise SDKError(
            code=ErrorCode.CALLER_SIGNER_UNAVAILABLE,
            stage="sign",
            retry=RetryHint.NEVER,
            retryable=False,
            message="fixture has no signer",
        )


class _ReceiptProvider:
    def __init__(self) -> None:
        self.list_calls = 0
        self.history_list_scope = "invocation.history.list"

    def receipt_history_list_authority_scope(self) -> str:
        return self.history_list_scope

    def list(self, request):
        self.list_calls += 1
        return None

    def get(self, request):
        return None

    def trace(self, request):
        return None


class _ReceiptProviderWithoutScope:
    def list(self, request):
        return None

    def get(self, request):
        return None

    def trace(self, request):
        return None


class _Clock:
    def now_unix_ms(self) -> int:
        return 1000

    def new_idempotency_key(self) -> str:
        return "idem-1"

    def new_nonce_base64(self) -> str:
        return "AQIDBAUGBwgJCgsMDQ4PEA=="


def _intent(caller: str = "easynet:///r/example/agent/backend") -> InvocationIntent:
    return InvocationIntent(
        caller_identity=CallerIdentityRef(PrincipalRef(caller)),
        acting_principal=ActingPrincipalRef(
            PrincipalRef("easynet:///r/example/agent/backend")
        ),
        target=RuntimeTargetRef("easynet:///r/example/device/dev-a"),
        ability=AbilityRef("invocation.history.list"),
        subject=SubjectRef(
            "easynet:///r/example/resource/user.alice/session/session-1",
            "fixture",
        ),
        call_mode="rpc",
        arguments={"limit": 10},
        deadline_unix_ms=2000,
        idempotency_key="idem-1",
        causal_context={"form": "none"},
    )


def _session_authority(override: dict[str, object] | None = None) -> SessionAuthority:
    payload = {
        "issuer_ura": "easynet:///r/example/agent/backend",
        "session_id": "session-1",
        "session_owner_user_id": "alice",
        "creator_principal_id": "easynet:///r/example/agent/backend",
        "callee_ura": "easynet:///r/example/device/dev-a",
        "subject_ura": "easynet:///r/example/resource/user.alice/session/session-1",
        "audience": "easynet:///r/example/device/dev-a",
        "scopes": ["invocation.history.*"],
        "allowed_actions": ["read"],
        "allowed_followup_abilities": ["invocation.history.list"],
        "issued_at_ms": 1000,
        "expires_at_ms": 2000,
    }
    if override:
        payload.update(override)
    return SessionAuthority.from_metadata(_authority_metadata(payload, b"session-signature"))


def _authority_metadata(payload: dict[str, object], signature: bytes) -> str:
    raw = json.dumps(
        {
            "payload": payload,
            "signature": base64.b64encode(signature).decode("ascii"),
        },
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")
    return base64.b64encode(raw).decode("ascii")


if __name__ == "__main__":
    unittest.main()
