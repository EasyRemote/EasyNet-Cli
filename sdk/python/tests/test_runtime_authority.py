from __future__ import annotations

from dataclasses import replace

import pytest

import easynet_sdk
from easynet_sdk.axon_addressing import AddressingClient, AxonAddressingTransport
from easynet_sdk.authority import (
    DELEGATION_METADATA_KEY,
    SESSION_AUTHORITY_METADATA_KEY,
    DelegationProof,
    SessionAuthority,
)
from easynet_sdk.invocation import InvocationBuilder
from easynet_sdk.runtime_authority import LocalRuntimeAuthorityProvider


class _Signer:
    def __init__(self) -> None:
        self.signed: list[bytes] = []

    def sign_canonical(self, canonical_bytes: bytes) -> bytes:
        self.signed.append(canonical_bytes)
        return bytes(range(64))


def _provider(signer: _Signer | None = None) -> LocalRuntimeAuthorityProvider:
    signer = signer or _Signer()
    return LocalRuntimeAuthorityProvider(
        AddressingClient(AxonAddressingTransport()),
        signer_loader=lambda owner_ura: signer,
        clock_ms=lambda: 1_000,
        authority_ttl_ms=60_000,
    )


def _draft(*, caller: str, subject: str):
    return (
        InvocationBuilder()
        .with_caller_ura(caller)
        .with_callee_ura(
            "easynet:///r/example/agent/device.dev-a.runtime-introspection"
        )
        .with_descriptor_ref(
            "easynet:///r/example/ability/system-agent.dev-a.runtime-introspection.meta.list_resources@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read"
        )
        .with_subject_ura(subject)
        .with_nonce_base64("AQIDBAUGBwgJCgsMDQ4PEA==")
        .with_causal_context({"form": "none"})
        .with_content_type("application/json")
        .with_json_args({})
        .build()
    )


def test_local_runtime_authority_binds_same_user_resource_subject() -> None:
    signer = _Signer()
    draft = _provider(signer).bind(
        _draft(
            caller="easynet:///r/example/user/alice",
            subject="easynet:///r/example/resource/user.alice/runtime-state/read",
        )
    )

    metadata = draft.metadata
    assert DELEGATION_METADATA_KEY in metadata
    proof = DelegationProof.from_metadata(metadata[DELEGATION_METADATA_KEY])
    assert proof.issuer_ura == "easynet:///r/example/user/alice"
    assert proof.caller_ura == "easynet:///r/example/user/alice"
    assert proof.subject_ura == (
        "easynet:///r/example/resource/user.alice/runtime-state/read"
    )
    assert (
        proof.audience
        == "easynet:///r/example/agent/device.dev-a.runtime-introspection"
    )
    assert proof.scopes == ("meta.list_resources",)
    assert proof.issued_at_ms == 1_000
    assert proof.expires_at_ms == 61_000
    assert signer.signed


def test_local_runtime_authority_rejects_cross_user_resource_subject() -> None:
    with pytest.raises(easynet_sdk.SDKError) as exc_info:
        _provider().bind(
            _draft(
                caller="easynet:///r/example/user/alice",
                subject="easynet:///r/example/resource/user.bob/runtime-state/read",
            )
        )

    assert exc_info.value.code == easynet_sdk.ErrorCode.AUTHORITY_SUBJECT_MISMATCH


def test_local_runtime_authority_binds_local_device_resource_session() -> None:
    signer = _Signer()
    draft = _draft(
        caller="easynet:///r/example/user/alice",
        subject="easynet:///r/example/resource/device.dev-a/streams/display.main",
    )

    bound = _provider(signer).bind(draft)

    authority = SessionAuthority.from_metadata(
        bound.metadata[SESSION_AUTHORITY_METADATA_KEY]
    )
    assert authority.issuer_ura == "easynet:///r/example/user/alice"
    assert authority.session_owner_user_id == "alice"
    assert authority.creator_principal_id == "easynet:///r/example/user/alice"
    assert authority.subject_ura == draft.subject_ura
    assert authority.audience == draft.callee_ura
    assert authority.scopes == ("meta.list_resources",)
    assert authority.allowed_actions == ("read",)
    assert authority.allowed_followup_abilities == ("meta.list_resources",)
    assert authority.session_id.startswith("invoke-")
    assert signer.signed


def test_local_runtime_authority_rejects_cross_device_resource_session() -> None:
    with pytest.raises(easynet_sdk.SDKError) as exc_info:
        _provider().bind(
            _draft(
                caller="easynet:///r/example/user/alice",
                subject="easynet:///r/example/resource/device.dev-b/streams/display.main",
            )
        )

    assert exc_info.value.code == easynet_sdk.ErrorCode.AUTHORITY_SUBJECT_MISMATCH


def test_local_runtime_authority_binds_same_user_agent_subject() -> None:
    draft = _provider().bind(
        _draft(
            caller="easynet:///r/example/user/alice",
            subject="easynet:///r/example/agent/alice.worker",
        )
    )

    proof = DelegationProof.from_metadata(draft.metadata[DELEGATION_METADATA_KEY])
    assert proof.subject_ura == "easynet:///r/example/agent/alice.worker"
    assert proof.scopes == ("meta.list_resources",)


def test_local_runtime_authority_rejects_foreign_user_agent_subject() -> None:
    with pytest.raises(easynet_sdk.SDKError) as exc_info:
        _provider().bind(
            _draft(
                caller="easynet:///r/example/user/alice",
                subject="easynet:///r/example/agent/bob.worker",
            )
        )

    assert exc_info.value.code == easynet_sdk.ErrorCode.AUTHORITY_SUBJECT_MISMATCH


def test_local_runtime_authority_preserves_existing_authority_metadata() -> None:
    draft = replace(
        _draft(
            caller="easynet:///r/example/user/alice",
            subject="easynet:///r/example/agent/alice.worker",
        ),
        metadata={DELEGATION_METADATA_KEY: "already-bound"},
    )

    assert _provider().bind(draft) is draft
