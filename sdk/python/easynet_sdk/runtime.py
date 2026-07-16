"""Runtime Core prepare and submit facade."""

from __future__ import annotations

import json
from dataclasses import dataclass, field, replace
from typing import Any, Mapping, Optional, Protocol, cast, runtime_checkable

from .errors import ErrorCode, RetryHint, SDKError
from .bidi import BidiSession, BidiStreamDescriptor, BidiTransport
from .invocation import InvocationBuilder, InvocationDraft
from .stream import StreamHandle, StreamTransport
from .signing import (
    PreparedInvocation,
    SignedInvocation,
    Signer,
    SigningMaterial,
    signing_material_from_prepare_json,
)


@runtime_checkable
class RuntimeTransport(Protocol):
    """Narrow transport seam owned by the application integration layer."""

    def invoke(self, draft_json: bytes) -> bytes: ...

    def open_stream(self, draft_json: bytes) -> tuple[StreamTransport, bytes]: ...

    def open_bidi(
        self, draft_json: bytes, streams_json: bytes
    ) -> tuple[BidiTransport, bytes]: ...

    def prepare(self, draft_json: bytes, options_json: bytes) -> bytes: ...

    def submit_signed(self, signed_json: bytes) -> bytes: ...

    def await_handle(self, control: "InvocationControlCapability") -> bytes: ...

    def cancel_handle(
        self, control: "InvocationControlCapability", reason: str
    ) -> bytes: ...

    def handle_events(self, control: "InvocationControlCapability") -> bytes: ...

    def free_handle(self, control: "InvocationControlCapability") -> None: ...

    def close(self) -> None: ...


@runtime_checkable
class DescriptorResolverTransport(Protocol):
    """Optional provider seam for runtime-bound descriptor resolution."""

    def resolve_descriptor_ref(self, request_json: bytes) -> bytes: ...


@dataclass(frozen=True)
class PrepareOptions:
    """Daemon-owned prepare policy knobs."""

    expires_in_ms: int = 0
    signer_id: str = ""
    policy_ref: str = ""
    local_daemon_signing: bool = False

    def to_json_dict(self) -> dict[str, object]:
        value: dict[str, object] = {}
        if self.expires_in_ms:
            value["expires_in_ms"] = self.expires_in_ms
        if self.signer_id:
            value["signer_id"] = self.signer_id

        if self.policy_ref:
            value["policy_ref"] = self.policy_ref
        if self.local_daemon_signing:
            value["local_daemon_signing"] = self.local_daemon_signing
        return value

    def to_json_bytes(self) -> bytes:
        return json.dumps(
            self.to_json_dict(), separators=(",", ":"), sort_keys=True
        ).encode("utf-8")


@dataclass(frozen=True)
class InvocationControlCapability:
    """Opaque authority for submitted invocation lifecycle control."""

    _handle_id: int = field(repr=False)
    _runtime_bound: bool = field(default=False, repr=False)

    @classmethod
    def _from_handle_id(cls, handle_id: int) -> "InvocationControlCapability":
        return cls._from_runtime_handle_id(handle_id)

    @classmethod
    def _from_runtime_handle_id(cls, handle_id: int) -> "InvocationControlCapability":
        if handle_id <= 0:
            raise _invalid_runtime("invocation control capability is required")
        return cls(_handle_id=handle_id, _runtime_bound=True)

    @classmethod
    def _from_snapshot_handle_id(cls, handle_id: int) -> "InvocationControlCapability":
        if handle_id <= 0:
            raise _invalid_runtime("handle_id is required")
        return cls(_handle_id=handle_id, _runtime_bound=False)

    def _adapter_handle_id(self) -> int:
        return self._handle_id

    def _is_runtime_bound(self) -> bool:
        return self._handle_id > 0 and self._runtime_bound


@dataclass(frozen=True)
class InvocationHandleEvent:
    """Submitted invocation event projection."""

    sequence: int
    kind: str
    state: str
    terminal: bool
    reason: Optional[str] = None
    result: Optional[Mapping[str, object]] = None


@dataclass(frozen=True)
class InvocationHandle:
    """Submitted invocation observation handle projection."""

    control: InvocationControlCapability
    state: str
    terminal: bool
    events: tuple[InvocationHandleEvent, ...] = field(default_factory=tuple)
    result: Optional[Mapping[str, object]] = None
    _runtime: Any = field(default=None, compare=False, repr=False)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "InvocationHandle":
        return _invocation_handle_from_json(raw)

    @classmethod
    def _from_runtime_json(cls, raw: bytes | str) -> "InvocationHandle":
        return _invocation_handle_from_json(raw, runtime_bound=True)

    @classmethod
    def _from_json_with_control(
        cls, raw: bytes | str, control: InvocationControlCapability
    ) -> "InvocationHandle":
        return _invocation_handle_from_json(raw, expected_control=control)

    def await_result(self) -> "InvocationResult":
        """Await this submitted Invocation through its bound RuntimeClient."""

        return _require_runtime(self._runtime).await_result(self)

    def cancel(self, reason: str = "") -> "InvocationCancel":
        """Cancel this submitted Invocation through its bound RuntimeClient."""

        return _require_runtime(self._runtime).cancel(self, reason)

    def refresh_events(self) -> "InvocationHandle":
        """Fetch the latest handle event projection through the bound RuntimeClient."""

        return _require_runtime(self._runtime).events(self)

    def close(self) -> None:
        """Release this submitted Invocation handle through its bound RuntimeClient."""

        _require_runtime(self._runtime).close_handle(self)

    def _bind_runtime(self, runtime: object) -> "InvocationHandle":
        return replace(self, _runtime=runtime)

    def control_capability(self) -> InvocationControlCapability:
        if not self.control._is_runtime_bound():
            raise _invalid_runtime("runtime-bound invocation control capability is required")
        return self.control


def _invocation_handle_from_json(
    raw: bytes | str,
    *,
    expected_control: Optional[InvocationControlCapability] = None,
    runtime_bound: bool = False,
) -> InvocationHandle:
    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        decoded = json.loads(text)
    except Exception as exc:
        raise _invalid_runtime(
            f"decode invocation handle JSON: {exc}", exc
        ) from exc
    if not isinstance(decoded, dict):
        raise _invalid_runtime("invocation handle JSON must be an object")

    handle_id = _required_positive_int(decoded, "handle_id")
    if expected_control is not None:
        if expected_control._adapter_handle_id() != handle_id:
            raise _invalid_runtime(
                "handle_id does not match invocation control capability"
            )
        control = expected_control
    elif runtime_bound:
        control = InvocationControlCapability._from_runtime_handle_id(handle_id)
    else:
        control = InvocationControlCapability._from_snapshot_handle_id(handle_id)
    state = _required_string(decoded, "state")
    terminal = _required_bool(decoded, "terminal")
    raw_events = decoded.get("events", [])
    if not isinstance(raw_events, list):
        raise _invalid_runtime("events must be an array")
    events = tuple(_handle_event(item) for item in raw_events)
    result = _optional_mapping(decoded.get("result"), "result")
    return InvocationHandle(
        control=control,
        state=state,
        terminal=terminal,
        events=events,
        result=result,
    )


@dataclass(frozen=True)
class InvocationFailure:
    """Runtime failure embedded in a terminal invocation result."""

    code: str
    stage: str
    message: str = ""
    retryable: bool = False


@dataclass(frozen=True)
class RuntimeReceiptAgentBinding:
    ura: str = ""
    profile: str = ""


@dataclass(frozen=True)
class RuntimeReceiptSubjectBinding:
    ura: str = ""
    profile: str = ""


@dataclass(frozen=True)
class RuntimeReceiptEntityRef:
    kind: int = 0
    ura: str = ""
    profile: str = ""


@dataclass(frozen=True)
class RuntimeReceiptSignature:
    algorithm: str = ""
    signature_base64: str = ""
    key_id_hint: str = ""


@dataclass(frozen=True)
class RuntimeReceiptFailure:
    code: str = ""
    message: str = ""
    retryable: bool = False
    stage: int = 0
    security_class: int = 0


@dataclass(frozen=True)
class RuntimeReceiptUsage:
    tokens_in: int = 0
    tokens_out: int = 0
    duration_ms: int = 0
    external_calls: int = 0


@dataclass(frozen=True)
class RuntimeReceiptRef:
    receipt_hash_hex: str = ""
    receipt_ura: str = ""


@dataclass(frozen=True)
class RuntimeReceiptAuthorityProof:
    proof_type: str = ""
    binding_kind: str = ""
    proof_payload_base64: str = ""
    proof_hash_hex: str = ""
    issuer: Optional[RuntimeReceiptAgentBinding] = None
    signature: Optional[RuntimeReceiptSignature] = None
    admission_hook: str = ""


@dataclass(frozen=True)
class RuntimeReceipt:
    """Non-verifying Runtime Core terminal receipt projection."""

    raw: Mapping[str, object]
    receipt_id: str = ""
    receipt_ura: str = ""
    invocation_id: str = ""
    receipt_type: str = ""
    state: str = ""
    index: int = 0
    timestamp_unix_ms: int = 0
    prev_receipt_hash_hex: str = ""
    self_hash_hex: str = ""
    cleanup_complete: Optional[bool] = None
    reason: str = ""
    child_invocation_id: str = ""
    payload_base64: str = ""
    caller_binding: Optional[RuntimeReceiptAgentBinding] = None
    callee_binding: Optional[RuntimeReceiptAgentBinding] = None
    subject_binding: Optional[RuntimeReceiptSubjectBinding] = None
    invocation_nonce_base64: str = ""
    causal_binding_kind: str = ""
    causal_binding: Optional[Mapping[str, object]] = None
    callee_signature: Optional[RuntimeReceiptSignature] = None
    signer_binding: Optional[RuntimeReceiptAgentBinding] = None
    host_attestation_base64: str = ""
    authority_binding_kind: str = ""
    authority_binding: Optional[Mapping[str, object]] = None
    ability_binding: str = ""
    failure: Optional[RuntimeReceiptFailure] = None
    usage: Optional[RuntimeReceiptUsage] = None
    subject_ref: Optional[RuntimeReceiptEntityRef] = None
    descriptor_version: str = ""
    schema_hash_hex: str = ""
    impl_hash_hex: str = ""
    runtime_env: str = ""
    authority_proof: Optional[RuntimeReceiptAuthorityProof] = None
    input_hash_hex: str = ""
    output_hash_hex: str = ""
    parent_receipts: tuple[RuntimeReceiptRef, ...] = field(default_factory=tuple)

    @classmethod
    def from_mapping(cls, decoded: Mapping[str, object]) -> "RuntimeReceipt":
        return cls(
            raw=dict(decoded),
            receipt_id=_optional_string(decoded.get("receipt_id"), "receipt_id") or "",
            receipt_ura=_optional_string(decoded.get("receipt_ura"), "receipt_ura")
            or "",
            invocation_id=_optional_string(
                decoded.get("invocation_id"), "invocation_id"
            )
            or "",
            receipt_type=_optional_runtime_summary_text(
                decoded.get("receipt_type"), "receipt_type"
            )
            or "",
            state=_optional_runtime_summary_text(decoded.get("state"), "state") or "",
            index=_optional_non_negative_int(decoded.get("index"), "index"),
            timestamp_unix_ms=_optional_non_negative_int(
                decoded.get("timestamp_unix_ms"), "timestamp_unix_ms"
            ),
            prev_receipt_hash_hex=_optional_string(
                decoded.get("prev_receipt_hash_hex"), "prev_receipt_hash_hex"
            )
            or "",
            self_hash_hex=_optional_string(
                decoded.get("self_hash_hex"), "self_hash_hex"
            )
            or "",
            cleanup_complete=_optional_bool(
                decoded.get("cleanup_complete"), "cleanup_complete"
            ),
            reason=_optional_string(decoded.get("reason"), "reason") or "",
            child_invocation_id=_optional_string(
                decoded.get("child_invocation_id"), "child_invocation_id"
            )
            or "",
            payload_base64=_optional_string(
                decoded.get("payload_base64"), "payload_base64"
            )
            or "",
            caller_binding=_receipt_agent_binding(
                decoded.get("caller_binding"), "caller_binding"
            ),
            callee_binding=_receipt_agent_binding(
                decoded.get("callee_binding"), "callee_binding"
            ),
            subject_binding=_receipt_subject_binding(
                decoded.get("subject_binding"), "subject_binding"
            ),
            invocation_nonce_base64=_optional_string(
                decoded.get("invocation_nonce_base64"), "invocation_nonce_base64"
            )
            or "",
            causal_binding_kind=_optional_string(
                decoded.get("causal_binding_kind"), "causal_binding_kind"
            )
            or "",
            causal_binding=_optional_mapping(
                decoded.get("causal_binding"), "causal_binding"
            ),
            callee_signature=_receipt_signature(
                decoded.get("callee_signature"), "callee_signature"
            ),
            signer_binding=_receipt_agent_binding(
                decoded.get("signer_binding"), "signer_binding"
            ),
            host_attestation_base64=_optional_string(
                decoded.get("host_attestation_base64"), "host_attestation_base64"
            )
            or "",
            authority_binding_kind=_optional_string(
                decoded.get("authority_binding_kind"), "authority_binding_kind"
            )
            or "",
            authority_binding=_optional_mapping(
                decoded.get("authority_binding"), "authority_binding"
            ),
            ability_binding=_optional_string(
                decoded.get("ability_binding"), "ability_binding"
            )
            or "",
            failure=_receipt_failure(decoded.get("failure"), "failure"),
            usage=_receipt_usage(decoded.get("usage"), "usage"),
            subject_ref=_receipt_entity_ref(decoded.get("subject_ref"), "subject_ref"),
            descriptor_version=_optional_string(
                decoded.get("descriptor_version"), "descriptor_version"
            )
            or "",
            schema_hash_hex=_optional_string(
                decoded.get("schema_hash_hex"), "schema_hash_hex"
            )
            or "",
            impl_hash_hex=_optional_string(
                decoded.get("impl_hash_hex"), "impl_hash_hex"
            )
            or "",
            runtime_env=_optional_string(decoded.get("runtime_env"), "runtime_env")
            or "",
            authority_proof=_receipt_authority_proof(
                decoded.get("authority_proof"), "authority_proof"
            ),
            input_hash_hex=_optional_string(
                decoded.get("input_hash_hex"), "input_hash_hex"
            )
            or "",
            output_hash_hex=_optional_string(
                decoded.get("output_hash_hex"), "output_hash_hex"
            )
            or "",
            parent_receipts=_receipt_refs(decoded.get("parent_receipts")),
        )

    @classmethod
    def from_required_mapping(cls, decoded: Mapping[str, object]) -> "RuntimeReceipt":
        """Decode and validate a complete daemon runtime receipt summary."""

        if not isinstance(decoded, Mapping):
            raise _invalid_runtime("runtime receipt summary must be an object")
        receipt = cls.from_mapping(decoded)
        if not receipt.invocation_id:
            raise _invalid_runtime("runtime receipt summary is missing invocation_id")
        if not receipt.receipt_type:
            raise _invalid_runtime("runtime receipt summary is missing receipt_type")
        receipt.prev_receipt_hash()
        receipt.self_receipt_hash()
        return receipt

    def has_causal_anchor(self) -> bool:
        """Return whether daemon/Axon supplied enough facts for causal linkage."""

        return bool(self.receipt_ura and self.self_hash_hex)

    def prev_receipt_hash(self) -> bytes:
        """Return the validated previous receipt hash bytes."""

        return _runtime_receipt_hash(
            self.prev_receipt_hash_hex, "prev_receipt_hash_hex"
        )

    def self_receipt_hash(self) -> bytes:
        """Return the validated self receipt hash bytes."""

        return _runtime_receipt_hash(self.self_hash_hex, "self_hash_hex")

    def to_json_dict(self) -> dict[str, object]:
        return dict(self.raw)


@dataclass(frozen=True)
class InvocationResult:
    """Unary invocation terminal result projection."""

    ok: bool
    tuple: InvocationDraft
    terminal_state: str
    output_content_type: str = ""
    output_base64: str = ""
    output_json: Any = None
    selected_node_id: str = ""
    scheduling_reason: str = ""
    elapsed_ms: int = 0
    error: Optional[InvocationFailure] = None
    admission_receipt: Optional[Mapping[str, object]] = None
    admission_receipt_summary: Optional[RuntimeReceipt] = None
    terminal_receipt: Optional[Mapping[str, object]] = None
    terminal_receipt_summary: Optional[RuntimeReceipt] = None

    @classmethod
    def from_json(cls, raw: bytes | str) -> "InvocationResult":
        try:
            text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
            decoded = json.loads(text)
        except Exception as exc:
            raise _invalid_runtime(
                f"decode invocation result JSON: {exc}", exc
            ) from exc
        if not isinstance(decoded, dict):
            raise _invalid_runtime("invocation result JSON must be an object")
        ok = _required_bool(decoded, "ok")
        tuple_value = _required_mapping(decoded, "tuple")
        draft = InvocationDraft.from_json(json.dumps(tuple_value))
        terminal_state = _required_string(decoded, "terminal_state")
        elapsed_ms = _optional_non_negative_int(decoded.get("elapsed_ms"), "elapsed_ms")
        failure = _failure(decoded.get("error"))
        if ok and failure is not None:
            raise _invalid_runtime("ok result must not include error")
        if not ok and failure is None:
            raise _invalid_runtime("failed result must include error")
        admission_receipt = _optional_mapping(
            decoded.get("admission_receipt"), "admission_receipt"
        )
        terminal_receipt = _optional_mapping(
            decoded.get("terminal_receipt"), "terminal_receipt"
        )
        admission_receipt_summary = (
            RuntimeReceipt.from_mapping(admission_receipt)
            if admission_receipt
            else None
        )
        terminal_receipt_summary = (
            RuntimeReceipt.from_mapping(terminal_receipt) if terminal_receipt else None
        )
        return cls(
            ok=ok,
            tuple=draft,
            terminal_state=terminal_state,
            output_content_type=_optional_string(
                decoded.get("output_content_type"), "output_content_type"
            )
            or "",
            output_base64=_optional_string(
                decoded.get("output_base64"), "output_base64"
            )
            or "",
            output_json=decoded.get("output_json"),
            selected_node_id=_optional_string(
                decoded.get("selected_node_id"), "selected_node_id"
            )
            or "",
            scheduling_reason=_optional_string(
                decoded.get("scheduling_reason"), "scheduling_reason"
            )
            or "",
            elapsed_ms=elapsed_ms,
            error=failure,
            admission_receipt=admission_receipt,
            admission_receipt_summary=admission_receipt_summary,
            terminal_receipt=terminal_receipt,
            terminal_receipt_summary=terminal_receipt_summary,
        )


@dataclass(frozen=True)
class InvocationCancel:
    """Daemon cancellation outcome for a submitted handle."""

    control: InvocationControlCapability
    request_accepted: bool
    deduplicated: bool
    cancelled: bool
    state: str
    terminal: bool

    @classmethod
    def from_json(cls, raw: bytes | str) -> "InvocationCancel":
        return _invocation_cancel_from_json(raw)

    @classmethod
    def _from_json_with_control(
        cls, raw: bytes | str, control: InvocationControlCapability
    ) -> "InvocationCancel":
        return _invocation_cancel_from_json(raw, expected_control=control)


def _invocation_cancel_from_json(
    raw: bytes | str,
    *,
    expected_control: Optional[InvocationControlCapability] = None,
) -> InvocationCancel:
    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
        decoded = json.loads(text)
    except Exception as exc:
        raise _invalid_runtime(
            f"decode invocation cancel JSON: {exc}", exc
        ) from exc
    if not isinstance(decoded, dict):
        raise _invalid_runtime("invocation cancel JSON must be an object")
    handle_id = _required_positive_int(decoded, "handle_id")
    if expected_control is not None:
        if expected_control._adapter_handle_id() != handle_id:
            raise _invalid_runtime("handle_id does not match invocation control capability")
        control = expected_control
    else:
        control = InvocationControlCapability._from_snapshot_handle_id(handle_id)
    return InvocationCancel(
        control=control,
        request_accepted=_required_bool(decoded, "request_accepted"),
        deduplicated=_required_bool(decoded, "deduplicated"),
        cancelled=_required_bool(decoded, "cancelled"),
        state=_required_string(decoded, "state"),
        terminal=_required_bool(decoded, "terminal"),
    )


class RuntimeClient:
    """Runtime Core invocation facade over an application transport."""

    def __init__(self, transport: RuntimeTransport) -> None:
        if transport is None:
            raise _invalid_runtime_client("runtime transport is required")
        self._transport = transport
        self._closed = False

    def new_invocation(self) -> InvocationBuilder:
        """Create a mutable Invocation builder bound to this RuntimeClient."""

        self._require_open()
        return InvocationBuilder()._bind_runtime(self)

    def resolve_descriptor_ref(
        self,
        *,
        callee_ura: str,
        ability: str,
        call_mode: str = "rpc",
        caller_ura: str = "",
        subject_ura: str = "",
    ) -> str:
        """Resolve a runtime-governed AbilityDescriptorRef through the provider."""

        transport = self._require_open()
        if not isinstance(transport, DescriptorResolverTransport):
            raise _invalid_runtime_client(
                "runtime transport does not expose descriptor resolution"
            )
        call_mode = call_mode.strip() or "rpc"
        request = {
            "callee_ura": callee_ura,
            "ability": ability,
            "call_mode": call_mode,
        }
        if caller_ura:
            request["caller_ura"] = caller_ura
        if subject_ura:
            request["subject_ura"] = subject_ura
        try:
            raw = transport.resolve_descriptor_ref(
                json.dumps(request, separators=(",", ":"), sort_keys=True).encode(
                    "utf-8"
                )
            )
            decoded = json.loads(raw.decode("utf-8") if isinstance(raw, bytes) else raw)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("resolve descriptor_ref transport failed", exc) from exc
        if not isinstance(decoded, Mapping):
            raise _invalid_runtime("descriptor_ref resolution must return an object")
        return _required_string(decoded, "descriptor_ref")

    def invoke(self, draft: InvocationDraft) -> InvocationResult:
        transport = self._require_open()
        try:
            raw = transport.invoke(draft.to_json().encode("utf-8"))
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("invoke transport failed", exc) from exc
        return InvocationResult.from_json(raw)

    def invoke_builder(self, builder: InvocationBuilder) -> InvocationResult:
        """Invoke a builder and consume it only after dispatch succeeds."""

        if builder is None:
            raise _invalid_runtime("invocation builder is required")
        draft = builder.inspect()
        result = self.invoke(draft)
        builder._consume()
        return result

    def invoke_stream(self, draft: InvocationDraft) -> StreamHandle:
        transport = self._require_open()
        try:
            stream_transport, open_json = transport.open_stream(
                draft.to_json().encode("utf-8")
            )
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("open stream transport failed", exc) from exc
        return StreamHandle.from_json(stream_transport, open_json)

    def open_bidi(
        self,
        draft: InvocationDraft,
        streams: tuple[BidiStreamDescriptor, ...],
    ) -> BidiSession:
        transport = self._require_open()
        try:
            streams_json = json.dumps(
                [stream.to_json_dict() for stream in streams],
                separators=(",", ":"),
                sort_keys=True,
            ).encode("utf-8")
            bidi_transport, open_json = transport.open_bidi(
                draft.to_json().encode("utf-8"),
                streams_json,
            )
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("open bidi transport failed", exc) from exc
        return BidiSession.from_json(bidi_transport, open_json)

    def prepare(
        self,
        draft: InvocationDraft,
        options: PrepareOptions = PrepareOptions(),
    ) -> tuple[PreparedInvocation, SigningMaterial]:
        transport = self._require_open()
        try:
            draft_json = draft.to_json().encode("utf-8")
            options_json = options.to_json_bytes()
            raw = transport.prepare(draft_json, options_json)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("prepare transport failed", exc) from exc
        prepared = PreparedInvocation.from_json(raw)._bind_runtime(self)
        return prepared, prepared.signing_material

    def prepare_signing_material(
        self,
        draft: InvocationDraft,
        options: PrepareOptions = PrepareOptions(),
    ) -> SigningMaterial:
        """Return canonical signing material without retaining a native handle."""
        transport = self._require_open()
        options_json = options.to_json_dict()
        options_json["material_only"] = True
        try:
            raw = transport.prepare(
                draft.to_json().encode("utf-8"),
                json.dumps(options_json, separators=(",", ":"), sort_keys=True).encode(
                    "utf-8"
                ),
            )
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error(
                "prepare signing material transport failed", exc
            ) from exc
        return signing_material_from_prepare_json(raw)

    def prepare_builder(
        self,
        builder: InvocationBuilder,
        options: PrepareOptions = PrepareOptions(),
    ) -> tuple[PreparedInvocation, SigningMaterial]:
        """Prepare a builder and consume it only after prepare succeeds."""

        if builder is None:
            raise _invalid_runtime("invocation builder is required")
        draft = builder.inspect()
        prepared, material = self.prepare(draft, options)
        builder._consume()
        return prepared, material

    def prepare_and_sign(
        self,
        draft: InvocationDraft,
        signer: Signer,
        options: PrepareOptions = PrepareOptions(),
    ) -> tuple[SignedInvocation, SigningMaterial]:
        """Prepare canonical material and return an inspectable signed envelope."""

        if signer is None:
            raise _invalid_runtime("signer is required")
        prepared, material = self.prepare(draft, options)
        return signer.sign(prepared)._bind_runtime(self), material

    def submit_signed(self, signed: SignedInvocation) -> InvocationHandle:
        transport = self._require_open()
        if not signed.submit_ready():
            raise _invalid_runtime("signed invocation is not submit-ready")
        try:
            raw = transport.submit_signed(signed.to_json().encode("utf-8"))
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("submit signed transport failed", exc) from exc
        return InvocationHandle._from_runtime_json(raw)._bind_runtime(self)

    def await_result(self, handle: InvocationHandle) -> InvocationResult:
        transport = self._require_open()
        _require_handle(handle)
        try:
            raw = transport.await_handle(handle.control_capability())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("await handle transport failed", exc) from exc
        return InvocationResult.from_json(raw)

    def cancel(self, handle: InvocationHandle, reason: str = "") -> InvocationCancel:
        transport = self._require_open()
        _require_handle(handle)
        control = handle.control_capability()
        try:
            raw = transport.cancel_handle(control, reason)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("cancel handle transport failed", exc) from exc
        return InvocationCancel._from_json_with_control(raw, control)

    def events(self, handle: InvocationHandle) -> InvocationHandle:
        transport = self._require_open()
        _require_handle(handle)
        control = handle.control_capability()
        try:
            raw = transport.handle_events(control)
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("handle events transport failed", exc) from exc
        return InvocationHandle._from_json_with_control(raw, control)._bind_runtime(self)

    def close_handle(self, handle: InvocationHandle) -> None:
        transport = self._require_open()
        _require_handle(handle)
        try:
            transport.free_handle(handle.control_capability())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("free handle transport failed", exc) from exc

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        try:
            self._transport.close()
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("runtime close transport failed", exc) from exc

    def _require_open(self) -> RuntimeTransport:
        if self._closed:
            raise _invalid_runtime_client("runtime client is closed")
        return self._transport


def _require_handle(handle: InvocationHandle) -> None:
    handle.control_capability()


def _handle_event(value: object) -> InvocationHandleEvent:
    if not isinstance(value, dict):
        raise _invalid_runtime("event must be an object")
    return InvocationHandleEvent(
        sequence=_required_positive_int(value, "sequence"),
        kind=_required_string(value, "kind"),
        state=_required_string(value, "state"),
        terminal=_required_bool(value, "terminal"),
        reason=_optional_string(value.get("reason"), "reason"),
        result=_optional_mapping(value.get("result"), "result"),
    )


def _failure(value: object) -> Optional[InvocationFailure]:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise _invalid_runtime("error must be an object or null")
    return InvocationFailure(
        code=_required_string(value, "code"),
        stage=_required_string(value, "stage"),
        message=_optional_string(value.get("message"), "message") or "",
        retryable=_optional_bool(value.get("retryable"), "retryable") or False,
    )


def _required_mapping(
    decoded: Mapping[str, object], field_name: str
) -> Mapping[str, object]:
    value = decoded.get(field_name)
    if not isinstance(value, dict):
        raise _invalid_runtime(f"{field_name} must be an object")
    return value


def _required_positive_int(decoded: Mapping[str, object], field_name: str) -> int:
    value = decoded.get(field_name)
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise _invalid_runtime(f"{field_name} is required")
    return value


def _optional_non_negative_int(value: object, field_name: str) -> int:
    if value is None:
        return 0
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_runtime(f"{field_name} must be a non-negative integer")
    return value


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_runtime(f"{field_name} is required")
    return value


def _required_bool(decoded: Mapping[str, object], field_name: str) -> bool:
    value = decoded.get(field_name)
    if not isinstance(value, bool):
        raise _invalid_runtime(f"{field_name} must be a boolean")
    return value


def _optional_bool(value: object, field_name: str) -> Optional[bool]:
    if value is None:
        return None
    if not isinstance(value, bool):
        raise _invalid_runtime(f"{field_name} must be a boolean or null")
    return value


def _optional_string(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid_runtime(f"{field_name} must be a string or null")
    return value


def _optional_runtime_summary_text(value: object, field_name: str) -> Optional[str]:
    if value is None:
        return None
    if isinstance(value, bool):
        raise _invalid_runtime(f"{field_name} must be a string, integer, or null")
    if isinstance(value, (str, int)):
        return str(value)
    raise _invalid_runtime(f"{field_name} must be a string, integer, or null")


def _runtime_receipt_hash(value: object, field_name: str) -> bytes:
    if not isinstance(value, str) or not value:
        raise _invalid_runtime(f"{field_name} is required")
    try:
        decoded = bytes.fromhex(value)
    except ValueError as error:
        raise _invalid_runtime(f"{field_name} must be hexadecimal", error) from error
    if len(decoded) != 32:
        raise _invalid_runtime(f"{field_name} must be exactly 32 bytes")
    return decoded


def _optional_mapping(value: object, field_name: str) -> Optional[Mapping[str, object]]:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise _invalid_runtime(f"{field_name} must be an object or null")
    return dict(value)


def _receipt_mapping(value: object, field_name: str) -> Optional[Mapping[str, object]]:
    if value is None:
        return None
    if not isinstance(value, Mapping):
        raise _invalid_runtime(f"{field_name} must be an object or null")
    return value


def _receipt_agent_binding(
    value: object, field_name: str
) -> Optional[RuntimeReceiptAgentBinding]:
    decoded = _receipt_mapping(value, field_name)
    if decoded is None:
        return None
    return RuntimeReceiptAgentBinding(
        ura=_optional_string(decoded.get("ura"), f"{field_name}.ura") or "",
        profile=_optional_string(decoded.get("profile"), f"{field_name}.profile") or "",
    )


def _receipt_subject_binding(
    value: object, field_name: str
) -> Optional[RuntimeReceiptSubjectBinding]:
    decoded = _receipt_mapping(value, field_name)
    if decoded is None:
        return None
    return RuntimeReceiptSubjectBinding(
        ura=_optional_string(decoded.get("ura"), f"{field_name}.ura") or "",
        profile=_optional_string(decoded.get("profile"), f"{field_name}.profile") or "",
    )


def _receipt_entity_ref(
    value: object, field_name: str
) -> Optional[RuntimeReceiptEntityRef]:
    decoded = _receipt_mapping(value, field_name)
    if decoded is None:
        return None
    return RuntimeReceiptEntityRef(
        kind=_optional_non_negative_int(decoded.get("kind"), f"{field_name}.kind"),
        ura=_optional_string(decoded.get("ura"), f"{field_name}.ura") or "",
        profile=_optional_string(decoded.get("profile"), f"{field_name}.profile") or "",
    )


def _receipt_signature(
    value: object, field_name: str
) -> Optional[RuntimeReceiptSignature]:
    decoded = _receipt_mapping(value, field_name)
    if decoded is None:
        return None
    return RuntimeReceiptSignature(
        algorithm=_optional_string(decoded.get("algorithm"), f"{field_name}.algorithm")
        or "",
        signature_base64=_optional_string(
            decoded.get("signature_base64"), f"{field_name}.signature_base64"
        )
        or "",
        key_id_hint=_optional_string(
            decoded.get("key_id_hint"), f"{field_name}.key_id_hint"
        )
        or "",
    )


def _receipt_failure(value: object, field_name: str) -> Optional[RuntimeReceiptFailure]:
    decoded = _receipt_mapping(value, field_name)
    if decoded is None:
        return None
    return RuntimeReceiptFailure(
        code=_optional_string(decoded.get("code"), f"{field_name}.code") or "",
        message=_optional_string(decoded.get("message"), f"{field_name}.message") or "",
        retryable=_optional_bool(decoded.get("retryable"), f"{field_name}.retryable")
        or False,
        stage=_optional_non_negative_int(decoded.get("stage"), f"{field_name}.stage"),
        security_class=_optional_non_negative_int(
            decoded.get("security_class"), f"{field_name}.security_class"
        ),
    )


def _receipt_usage(value: object, field_name: str) -> Optional[RuntimeReceiptUsage]:
    decoded = _receipt_mapping(value, field_name)
    if decoded is None:
        return None
    return RuntimeReceiptUsage(
        tokens_in=_optional_non_negative_int(
            decoded.get("tokens_in"), f"{field_name}.tokens_in"
        ),
        tokens_out=_optional_non_negative_int(
            decoded.get("tokens_out"), f"{field_name}.tokens_out"
        ),
        duration_ms=_optional_non_negative_int(
            decoded.get("duration_ms"), f"{field_name}.duration_ms"
        ),
        external_calls=_optional_non_negative_int(
            decoded.get("external_calls"), f"{field_name}.external_calls"
        ),
    )


def _receipt_ref(value: object, field_name: str) -> RuntimeReceiptRef:
    decoded = _receipt_mapping(value, field_name)
    if decoded is None:
        raise _invalid_runtime(f"{field_name} must be an object")
    return RuntimeReceiptRef(
        receipt_hash_hex=_optional_string(
            decoded.get("receipt_hash_hex"), f"{field_name}.receipt_hash_hex"
        )
        or "",
        receipt_ura=_optional_string(
            decoded.get("receipt_ura"), f"{field_name}.receipt_ura"
        )
        or "",
    )


def _receipt_refs(value: object) -> tuple[RuntimeReceiptRef, ...]:
    if value is None:
        return ()
    if not isinstance(value, list):
        raise _invalid_runtime("parent_receipts must be an array or null")
    return tuple(
        _receipt_ref(item, f"parent_receipts[{index}]")
        for index, item in enumerate(value)
    )


def _receipt_authority_proof(
    value: object, field_name: str
) -> Optional[RuntimeReceiptAuthorityProof]:
    decoded = _receipt_mapping(value, field_name)
    if decoded is None:
        return None
    return RuntimeReceiptAuthorityProof(
        proof_type=_optional_string(
            decoded.get("proof_type"), f"{field_name}.proof_type"
        )
        or "",
        binding_kind=_optional_string(
            decoded.get("binding_kind"), f"{field_name}.binding_kind"
        )
        or "",
        proof_payload_base64=_optional_string(
            decoded.get("proof_payload_base64"),
            f"{field_name}.proof_payload_base64",
        )
        or "",
        proof_hash_hex=_optional_string(
            decoded.get("proof_hash_hex"), f"{field_name}.proof_hash_hex"
        )
        or "",
        issuer=_receipt_agent_binding(decoded.get("issuer"), f"{field_name}.issuer"),
        signature=_receipt_signature(
            decoded.get("signature"), f"{field_name}.signature"
        ),
        admission_hook=_optional_string(
            decoded.get("admission_hook"), f"{field_name}.admission_hook"
        )
        or "",
    )


def _invalid_runtime_client(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="sdk",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )


def _invalid_runtime(message: str, cause: Optional[BaseException] = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="runtime",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )


def _transport_error(message: str, cause: BaseException) -> SDKError:
    return SDKError(
        code=ErrorCode.ROUTE_UNAVAILABLE,
        stage="transport",
        retry=RetryHint.SAFE,
        retryable=True,
        message=message,
        cause=cause,
    )


def _require_runtime(runtime: object | None) -> RuntimeClient:
    if runtime is None:
        raise SDKError(
            code=ErrorCode.INVALID_HANDLE,
            stage="runtime",
            retry=RetryHint.NEVER,
            retryable=False,
            message="invocation handle is not bound to a RuntimeClient",
        )
    return cast(RuntimeClient, runtime)
