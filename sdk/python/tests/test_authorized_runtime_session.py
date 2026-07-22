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
    RuntimeTargetRef,
    RuntimeClientSessionRuntimeProvider,
    RetryHint,
    SDKError,
    SessionAuthority,
    SigningMaterial,
    StaticCallerIdentityProvider,
    SubjectRef,
    is_code,
)


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

    def test_history_rejects_authority_subject_mismatch_before_receipt_provider(self) -> None:
        fixture = _SessionFixture()
        request = ReceiptListRequest(
            call=RuntimeCallContext(
                caller_ura="easynet:///r/example/agent/backend",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura="easynet:///r/example/device/dev-a",
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                authority=_session_authority(),
            ),
            limit=10,
        )

        with self.assertRaises(SDKError) as caught:
            fixture.session.history.list(request)

        self.assertTrue(is_code(caught.exception, ErrorCode.AUTHORITY_SUBJECT_MISMATCH))
        self.assertEqual(fixture.receipts.list_calls, 0)

    def test_history_rejects_owner_equivalent_subject_expansion_before_receipt_provider(self) -> None:
        fixture = _SessionFixture()
        request = ReceiptListRequest(
            call=RuntimeCallContext(
                caller_ura="easynet:///r/example/agent/backend",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura="easynet:///r/example/resource/user.alice/session/session-2",
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                authority=_session_authority(),
            ),
            limit=10,
        )

        with self.assertRaises(SDKError) as caught:
            fixture.session.history.list(request)

        self.assertTrue(is_code(caught.exception, ErrorCode.AUTHORITY_SUBJECT_MISMATCH))
        self.assertEqual(fixture.receipts.list_calls, 0)

    def test_history_allows_session_authority_with_exact_device_subject_filter(self) -> None:
        fixture = _SessionFixture()
        request = ReceiptListRequest(
            call=RuntimeCallContext(
                caller_ura="easynet:///r/example/agent/backend",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura="easynet:///r/example/resource/user.alice/session/session-1",
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

    def test_runtime_client_provider_rejects_unsigned_stream_downgrade(self) -> None:
        provider = RuntimeClientSessionRuntimeProvider(object())

        with self.assertRaises(SDKError) as stream_error:
            provider.open_stream(object())
        self.assertTrue(is_code(stream_error.exception, ErrorCode.PROVIDER_UNAVAILABLE))

        with self.assertRaises(SDKError) as bidi_error:
            provider.open_bidi(object(), ())
        self.assertTrue(is_code(bidi_error.exception, ErrorCode.PROVIDER_UNAVAILABLE))


class _SessionFixture:
    def __init__(self, identity: object | None = None) -> None:
        self.runtime = _RuntimeProvider()
        self.descriptor = _DescriptorProvider()
        self.authorization = _AuthorizationProvider()
        self.signer = _SignerProvider()
        self.identity = identity or StaticCallerIdentityProvider(
            CallerIdentityRef(PrincipalRef("easynet:///r/example/agent/backend"))
        )
        self.receipts = _ReceiptProvider()
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

    def list(self, request):
        self.list_calls += 1
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
