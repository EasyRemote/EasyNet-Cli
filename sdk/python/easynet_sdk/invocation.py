"""Runtime Core Invocation DTOs and builder."""

from __future__ import annotations

import base64
import binascii
import json
import os
from dataclasses import dataclass, field, replace
from typing import TYPE_CHECKING, Any, Mapping, Optional, cast

if TYPE_CHECKING:
    from .bidi import BidiSession, BidiStreamDescriptor
    from .runtime import InvocationResult, RuntimeClient
    from .runtime import PrepareOptions
    from .signing import PreparedInvocation, SigningMaterial
    from .stream import StreamHandle

from .authority import AuthorityMetadata, validate_authority_metadata
from .errors import ErrorCode, RetryHint, SDKError


def new_invocation_nonce_base64() -> str:
    """Return a fresh 16-byte Invocation nonce encoded for the shared DTO."""

    return base64.b64encode(os.urandom(16)).decode("ascii")


@dataclass(frozen=True)
class InvocationSignature:
    """Caller signature material attached to a signed Invocation."""

    algorithm: str
    signature_base64: str
    key_id_hint: Optional[str] = None
    signer_public_key_base64: Optional[str] = None

    def to_json_dict(self) -> dict[str, object]:
        value: dict[str, object] = {
            "algorithm": self.algorithm,
            "signature_base64": self.signature_base64,
        }
        if self.key_id_hint is not None:
            value["key_id_hint"] = self.key_id_hint
        if self.signer_public_key_base64 is not None:
            value["signer_public_key_base64"] = self.signer_public_key_base64
        return value


@dataclass(frozen=True)
class InvocationDraft:
    """Immutable complete Invocation tuple."""

    caller_ura: str
    callee_ura: str
    descriptor_ref: str
    subject_ura: str
    nonce_base64: str
    causal_context: Mapping[str, object]
    content_type: str
    args: Any = None
    arguments_base64: Optional[str] = None
    metadata: Mapping[str, object] = field(default_factory=dict)
    caller_signature: Optional[InvocationSignature] = None
    _has_args: bool = False
    _runtime: Any = field(default=None, compare=False, repr=False)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "InvocationDraft":
        """Decode and validate the shared Invocation JSON DTO."""

        try:
            text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
            decoded = json.loads(text)
        except Exception as exc:
            raise _invalid_invocation(f"decode invocation JSON: {exc}", exc) from exc
        if not isinstance(decoded, dict):
            raise _invalid_invocation("invocation JSON must be an object")
        _reject_unknown_fields(decoded)
        builder = InvocationBuilder()
        builder.with_caller_ura(_required_string(decoded, "caller_ura"))
        builder.with_callee_ura(_required_string(decoded, "callee_ura"))
        builder.with_descriptor_ref(_required_string(decoded, "descriptor_ref"))
        builder.with_subject_ura(_required_string(decoded, "subject_ura"))
        builder.with_nonce_base64(_required_string(decoded, "nonce_base64"))
        builder.with_content_type(_required_string(decoded, "content_type"))
        builder.with_causal_context(_required_object(decoded, "causal_context"))
        if "args" in decoded:
            builder.with_json_args(decoded["args"])
        if "arguments_base64" in decoded:
            builder.with_arguments_base64(_required_string(decoded, "arguments_base64"))
        if "metadata" in decoded:
            builder.with_metadata(_required_object(decoded, "metadata"))
        if "caller_signature" in decoded:
            signature = _required_object(decoded, "caller_signature")
            builder.with_caller_signature(
                InvocationSignature(
                    algorithm=_required_string(signature, "algorithm"),
                    signature_base64=_required_string(signature, "signature_base64"),
                    key_id_hint=_optional_string(signature.get("key_id_hint"), "key_id_hint"),
                    signer_public_key_base64=_optional_string(
                        signature.get("signer_public_key_base64"),
                        "signer_public_key_base64",
                    ),
                )
            )
        return builder.build()

    def to_json_dict(self) -> dict[str, object]:
        """Return the shared sdk/schemas/invocation.schema.json shape."""

        value: dict[str, object] = {
            "caller_ura": self.caller_ura,
            "callee_ura": self.callee_ura,
            "descriptor_ref": self.descriptor_ref,
            "subject_ura": self.subject_ura,
            "nonce_base64": self.nonce_base64,
            "causal_context": dict(self.causal_context),
            "content_type": self.content_type,
        }
        if self._has_args:
            value["args"] = self.args
        else:
            assert self.arguments_base64 is not None
            value["arguments_base64"] = self.arguments_base64
        if self.metadata is not None:
            value["metadata"] = dict(self.metadata)
        if self.caller_signature is not None:
            value["caller_signature"] = self.caller_signature.to_json_dict()
        return value

    def to_json(self) -> str:
        return json.dumps(self.to_json_dict(), separators=(",", ":"), sort_keys=True)

    def prepare(
        self, options: "PrepareOptions | None" = None
    ) -> tuple["PreparedInvocation", "SigningMaterial"]:
        """Prepare this draft through its bound RuntimeClient."""

        runtime = _require_runtime(self._runtime)
        if options is None:
            return runtime.prepare(self)
        return runtime.prepare(self, options)

    def invoke(self) -> "InvocationResult":
        """Submit this draft as one unary Invocation through its bound RuntimeClient."""

        return _require_runtime(self._runtime).invoke(self)

    def open_stream(self) -> "StreamHandle":
        """Open this draft as one server-stream Invocation."""

        return _require_runtime(self._runtime).invoke_stream(self)

    def open_bidi(
        self, streams: tuple["BidiStreamDescriptor", ...] = ()
    ) -> "BidiSession":
        """Open this draft as one bidirectional Invocation."""

        return _require_runtime(self._runtime).open_bidi(self, streams)

    def _bind_runtime(self, runtime: object) -> "InvocationDraft":
        return replace(self, _runtime=runtime)


class InvocationBuilder:
    """Mutable builder for a complete Invocation tuple."""

    def __init__(self) -> None:
        self._caller_ura: Optional[str] = None
        self._callee_ura: Optional[str] = None
        self._descriptor_ref: Optional[str] = None
        self._subject_ura: Optional[str] = None
        self._nonce_base64: Optional[str] = None
        self._causal_context: Optional[Mapping[str, object]] = None
        self._args: Any = None
        self._arguments_base64: Optional[str] = None
        self._content_type: Optional[str] = None
        self._metadata: Mapping[str, object] = {}
        self._caller_signature: Optional[InvocationSignature] = None
        self._has_args = False
        self._has_arguments = False
        self._consumed = False
        self._runtime: object | None = None

    def with_caller_ura(self, value: str) -> "InvocationBuilder":
        self._caller_ura = value
        return self

    def with_callee_ura(self, value: str) -> "InvocationBuilder":
        self._callee_ura = value
        return self

    def with_descriptor_ref(self, value: str) -> "InvocationBuilder":
        self._descriptor_ref = value
        return self

    def with_subject_ura(self, value: str) -> "InvocationBuilder":
        self._subject_ura = value
        return self

    def with_nonce_base64(self, value: str) -> "InvocationBuilder":
        self._nonce_base64 = value
        return self

    def with_causal_context(self, value: Mapping[str, object]) -> "InvocationBuilder":
        self._causal_context = dict(value)
        return self

    def with_json_args(self, value: object) -> "InvocationBuilder":
        self._args = value
        self._has_args = True
        return self

    def with_arguments_base64(self, value: str) -> "InvocationBuilder":
        self._arguments_base64 = value
        self._has_arguments = True
        return self

    def with_content_type(self, value: str) -> "InvocationBuilder":
        self._content_type = value
        return self

    def with_metadata(self, value: Mapping[str, object]) -> "InvocationBuilder":
        self._metadata = dict(value)
        return self

    def with_authority_metadata(
        self, value: AuthorityMetadata
    ) -> "InvocationBuilder":
        self._metadata = value.merge_into(self._metadata)
        return self

    def with_caller_signature(
        self, value: InvocationSignature
    ) -> "InvocationBuilder":
        self._caller_signature = value
        return self

    def build(self) -> InvocationDraft:
        draft = self._inspect_draft()
        self._consumed = True
        return draft

    def inspect(self) -> InvocationDraft:
        """Validate tuple completeness without consuming the builder handle."""

        return self._inspect_draft()

    def prepare(
        self, options: "PrepareOptions | None" = None
    ) -> tuple["PreparedInvocation", "SigningMaterial"]:
        """Prepare this builder through its bound RuntimeClient."""

        runtime = _require_runtime(self._runtime)
        if options is None:
            return runtime.prepare_builder(self)
        return runtime.prepare_builder(self, options)

    def invoke(self) -> "InvocationResult":
        """Submit this builder as one unary Invocation through its bound RuntimeClient."""

        return _require_runtime(self._runtime).invoke_builder(self)

    def _bind_runtime(self, runtime: object) -> "InvocationBuilder":
        if self._consumed:
            raise _invalid_handle("invocation builder handle is consumed")
        self._runtime = runtime
        return self

    def _consume(self) -> None:
        if self._consumed:
            raise _invalid_handle("invocation builder handle is consumed")
        self._consumed = True

    def _inspect_draft(self) -> InvocationDraft:
        if self._consumed:
            raise _invalid_handle("invocation builder handle is consumed")
        caller_ura = _required_builder_string(self._caller_ura, "caller_ura")
        callee_ura = _required_builder_string(self._callee_ura, "callee_ura")
        descriptor_ref = _required_builder_string(self._descriptor_ref, "descriptor_ref")
        subject_ura = _required_builder_string(self._subject_ura, "subject_ura")
        nonce_base64 = _required_builder_string(self._nonce_base64, "nonce_base64")
        content_type = _required_builder_string(self._content_type, "content_type")
        if self._causal_context is None:
            raise _invalid_invocation("causal_context is required")
        _validate_nonce_base64(nonce_base64)
        if self._has_args == self._has_arguments:
            raise _invalid_invocation("exactly one of args or arguments_base64 is required")
        if self._has_arguments:
            _required_builder_string(self._arguments_base64, "arguments_base64")
            _validate_base64_field(self._arguments_base64 or "", "arguments_base64")
        validate_authority_metadata(self._metadata)
        if self._caller_signature is not None:
            _required_builder_string(
                self._caller_signature.algorithm, "caller_signature.algorithm"
            )
            _required_builder_string(
                self._caller_signature.signature_base64,
                "caller_signature.signature_base64",
            )

        return InvocationDraft(
            caller_ura=caller_ura,
            callee_ura=callee_ura,
            descriptor_ref=descriptor_ref,
            subject_ura=subject_ura,
            nonce_base64=nonce_base64,
            causal_context=dict(self._causal_context),
            args=self._args,
            arguments_base64=self._arguments_base64,
            content_type=content_type,
            metadata=dict(self._metadata),
            caller_signature=self._caller_signature,
            _has_args=self._has_args,
            _runtime=self._runtime,
        )


def _validate_nonce_base64(value: str) -> None:
    raw = _validate_base64_field(value, "nonce_base64")
    if len(raw) != 16:
        raise _invalid_invocation("nonce_base64 must decode to 16 bytes")


def _validate_base64_field(value: str, field_name: str) -> bytes:
    try:
        raw = base64.b64decode(value, validate=True)
    except binascii.Error as exc:
        raise _invalid_invocation(f"{field_name} must be base64", exc) from exc
    return raw


def _required_builder_string(value: Optional[str], field_name: str) -> str:
    if value is None or value.strip() == "":
        raise _invalid_invocation(f"{field_name} is required")
    return value


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_invocation(f"{field_name} is required")
    return value


def _required_object(
    decoded: Mapping[str, object], field_name: str
) -> Mapping[str, object]:
    value = decoded.get(field_name)
    if not isinstance(value, dict):
        raise _invalid_invocation(f"{field_name} must be an object")
    return value


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_invocation(f"{field_name} must be a string or null")
    return value


def _reject_unknown_fields(decoded: Mapping[str, object]) -> None:
    allowed = {
        "caller_ura",
        "callee_ura",
        "descriptor_ref",
        "subject_ura",
        "nonce_base64",
        "causal_context",
        "args",
        "arguments_base64",
        "content_type",
        "metadata",
        "caller_signature",
    }
    for name in decoded:
        if name not in allowed:
            raise _invalid_invocation(f"{name} is not an invocation field")


def _require_runtime(runtime: object | None) -> "RuntimeClient":
    if runtime is None:
        raise SDKError(
            code=ErrorCode.INVALID_HANDLE,
            stage="invocation",
            retry=RetryHint.NEVER,
            retryable=False,
            message="invocation is not bound to a RuntimeClient",
        )
    return cast("RuntimeClient", runtime)


def _invalid_invocation(
    message: str, cause: Optional[BaseException] = None
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="build",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )


def _invalid_handle(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_HANDLE,
        stage="build",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )
