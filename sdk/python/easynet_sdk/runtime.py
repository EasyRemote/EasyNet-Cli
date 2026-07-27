"""Runtime Core prepare and submit facade."""

from __future__ import annotations

import base64
import json
from dataclasses import dataclass, field, fields, replace
from types import MappingProxyType
from typing import Any, Mapping, Optional, Protocol, cast, runtime_checkable

from axon_sdk.invocation import (
    AgentIdentity as _AxonAgentIdentity,
    AuthorityBinding as _AxonAuthorityBinding,
    AxonError as _AxonError,
    BootstrapAuthorityBody as _AxonBootstrapAuthorityBody,
    CalleeSignature as _AxonCalleeSignature,
    DelegationProofBody as _AxonDelegationProofBody,
    EntityRef as _AxonEntityRef,
    EntityRefKind as _AxonEntityRefKind,
    InvocationAuthorityProof as _AxonInvocationAuthorityProof,
    ReceiptProofFacts as _AxonReceiptProofFacts,
    ReceiptRef as _AxonReceiptRef,
    SessionAuthorityBody as _AxonSessionAuthorityBody,
    UraProfile as _AxonUraProfile,
)

from .errors import ErrorCode, RetryHint, SDKError
from ._identity_guards import contains_all_zero_principal
from ._receipt_projection import reject_retired_top_level_receipt_alias
from .bidi import BidiSession, BidiStreamDescriptor, BidiTransport
from .invocation import InvocationBuilder, InvocationDraft
from .invocation_state import InvocationLifecycleState
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
class RuntimeRecoveryTransport(Protocol):
    """Optional provider seam for bounded restart recovery."""

    def recover(self, request_json: bytes) -> bytes: ...


@runtime_checkable
class DescriptorResolverTransport(Protocol):
    """Optional provider seam for runtime-bound descriptor resolution."""

    def resolve_descriptor_ref(self, request_json: bytes) -> bytes: ...


@dataclass(frozen=True)
class PrepareOptions:
    """Runtime-owned prepare policy knobs."""

    expires_in_ms: int = 0
    signer_id: str = ""
    policy_ref: str = ""

    def to_json_dict(self) -> dict[str, object]:
        value: dict[str, object] = {}
        if self.expires_in_ms:
            value["expires_in_ms"] = self.expires_in_ms
        if self.signer_id:
            value["signer_id"] = self.signer_id

        if self.policy_ref:
            value["policy_ref"] = self.policy_ref
        return value

    def to_json_bytes(self) -> bytes:
        return json.dumps(
            self.to_json_dict(), separators=(",", ":"), sort_keys=True
        ).encode("utf-8")


@dataclass(frozen=True)
class RuntimeRecoveryRequest:
    """Bounded Runtime restart-recovery request."""

    recovery_id: str
    deadline_unix_ms: int
    max_invocations: int

    def to_json_dict(self) -> dict[str, object]:
        _required_text(self.recovery_id, "recovery_id")
        if self.deadline_unix_ms <= 0:
            raise _invalid_runtime("deadline_unix_ms is required")
        if self.max_invocations <= 0:
            raise _invalid_runtime("max_invocations is required")
        return {
            "deadline_unix_ms": self.deadline_unix_ms,
            "max_invocations": self.max_invocations,
            "recovery_id": self.recovery_id.strip(),
        }

    def to_json_bytes(self) -> bytes:
        return json.dumps(
            self.to_json_dict(), separators=(",", ":"), sort_keys=True
        ).encode("utf-8")


@dataclass(frozen=True)
class RuntimeRecoveryEvent:
    """Runtime restart-recovery event projection."""

    sequence: int
    kind: str
    terminal: bool
    invocation_id: str = ""
    state: str = ""
    receipt_ura: str = ""
    reason: str = ""


@dataclass(frozen=True)
class RuntimeRecoveryReport:
    """Provider proof that restart recovery reached runtime_started."""

    recovery_id: str
    state: str
    recovered_invocations: int
    reaped_orphans: int
    replayed_terminal_receipts: int
    bounded_scan: bool
    cleanup_complete: bool
    events: tuple[RuntimeRecoveryEvent, ...] = field(default_factory=tuple)

    @classmethod
    def from_json(cls, raw: bytes | str) -> "RuntimeRecoveryReport":
        try:
            text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
            decoded = json.loads(text)
        except Exception as exc:
            raise _invalid_runtime(
                f"decode runtime recovery report JSON: {exc}", exc
            ) from exc
        if not isinstance(decoded, dict):
            raise _invalid_runtime("runtime recovery report JSON must be an object")
        state = _required_string(decoded, "state")
        if state != "runtime_started":
            raise _invalid_runtime("runtime recovery state must be runtime_started")
        bounded_scan = _required_bool(decoded, "bounded_scan")
        if not bounded_scan:
            raise _invalid_runtime("bounded_scan must be true")
        cleanup_complete = _required_bool(decoded, "cleanup_complete")
        if not cleanup_complete:
            raise _invalid_runtime("cleanup_complete must be true")
        raw_events = decoded.get("events", [])
        if not isinstance(raw_events, list):
            raise _invalid_runtime("events must be an array")
        return cls(
            recovery_id=_required_string(decoded, "recovery_id"),
            state=state,
            recovered_invocations=_required_non_negative_int(
                decoded.get("recovered_invocations"), "recovered_invocations"
            ),
            reaped_orphans=_required_non_negative_int(
                decoded.get("reaped_orphans"), "reaped_orphans"
            ),
            replayed_terminal_receipts=_required_non_negative_int(
                decoded.get("replayed_terminal_receipts"),
                "replayed_terminal_receipts",
            ),
            bounded_scan=bounded_scan,
            cleanup_complete=cleanup_complete,
            events=tuple(_runtime_recovery_event(item) for item in raw_events),
        )


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
            raise _invalid_runtime(
                "runtime-bound invocation control capability is required"
            )
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
        raise _invalid_runtime(f"decode invocation handle JSON: {exc}", exc) from exc
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
    binding: Optional[Mapping[str, object]] = None
    proof_payload_base64: str = ""
    proof_hash_hex: str = ""
    issuer: Optional[RuntimeReceiptAgentBinding] = None
    signature: Optional[RuntimeReceiptSignature] = None
    admission_hook: str = ""


@dataclass(frozen=True)
class _DecodedRuntimeReceipt:
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

    def __post_init__(self) -> None:
        decoded = _decode_runtime_receipt_mapping(
            _mutable_runtime_receipt_projection(self.raw)
        )
        for decoded_field in fields(_DecodedRuntimeReceipt):
            name = decoded_field.name
            if getattr(self, name) != getattr(decoded, name):
                raise _invalid_runtime(
                    f"runtime receipt {name} does not match its raw projection"
                )
        self.validate_summary()

    @classmethod
    def from_mapping(cls, decoded: Mapping[str, object]) -> "RuntimeReceipt":
        """Decode the only accepted canonical runtime receipt projection."""

        raw = _immutable_runtime_receipt_projection(decoded, "runtime receipt")
        projection = _decode_runtime_receipt_mapping(
            _mutable_runtime_receipt_projection(raw)
        )
        return cls(raw=raw, **vars(projection))

    @classmethod
    def from_required_mapping(cls, decoded: Mapping[str, object]) -> "RuntimeReceipt":
        """Decode a complete receipt through the canonical strict constructor."""

        return cls.from_mapping(decoded)

    def validate_summary(self) -> None:
        """Validate required identity, lifecycle, hash, and proof facts."""

        if not self.invocation_id:
            raise _invalid_runtime("runtime receipt summary is missing invocation_id")
        if not self.receipt_type:
            raise _invalid_runtime("runtime receipt summary is missing receipt_type")
        if not self.state:
            raise _invalid_runtime("runtime receipt summary is missing state")
        lifecycle_state = self.lifecycle_state
        if self.receipt_type != _canonical_receipt_type(lifecycle_state):
            raise _invalid_runtime(
                "runtime receipt receipt_type does not match its lifecycle state"
            )
        self.prev_receipt_hash()
        self.self_receipt_hash()
        self.validate_proof_facts()

    @property
    def lifecycle_state(self) -> InvocationLifecycleState:
        """Return the fail-closed canonical lifecycle state."""

        try:
            state = InvocationLifecycleState.from_wire_name(self.state)
        except ValueError as error:
            raise _invalid_runtime(
                str(error),
                error,
                details={"reason": "invalid_lifecycle_state"},
            ) from error
        if state is InvocationLifecycleState.UNSPECIFIED:
            raise _invalid_runtime(
                "runtime receipt lifecycle state must not be UNSPECIFIED",
                details={"reason": "invalid_lifecycle_state"},
            )
        return state

    def has_causal_anchor(self) -> bool:
        """Return whether the runtime supplied enough facts for causal linkage."""

        return bool(self.receipt_ura and self.self_hash_hex)

    def prev_receipt_hash(self) -> bytes:
        """Return the validated previous receipt hash bytes."""

        return _runtime_receipt_hash(
            self.prev_receipt_hash_hex,
            "prev_receipt_hash_hex",
            allow_zero=True,
        )

    def self_receipt_hash(self) -> bytes:
        """Return the validated self receipt hash bytes."""

        return _runtime_receipt_hash(self.self_hash_hex, "self_hash_hex")

    def validate_proof_facts(self) -> None:
        """Reject receipt projections that omit canonical proof facts."""

        _require_runtime_receipt_required_keys(
            self.raw,
            "runtime_receipt",
            "receipt_ura",
            "invocation_id",
            "receipt_type",
            "state",
            "index",
            "timestamp_unix_ms",
            "prev_receipt_hash_hex",
            "self_hash_hex",
            "payload_base64",
            "payload_content_type",
            "cleanup_complete",
            "caller_binding",
            "callee_binding",
            "subject_binding",
            "invocation_nonce_base64",
            "causal_binding_kind",
            "causal_binding",
            "callee_signature",
            "signer_binding",
            "host_attestation_base64",
            "authority_binding_kind",
            "authority_binding",
            "ability_binding",
            "usage",
            "subject_ref",
            "descriptor_version",
            "schema_hash_hex",
            "impl_hash_hex",
            "runtime_env",
            "authority_proof",
            "input_hash_hex",
            "output_hash_hex",
            "parent_receipts",
        )
        _runtime_receipt_base64(
            self.raw.get("payload_base64"),
            "payload_base64",
            allow_empty=True,
        )
        _required_receipt_text(self.raw.get("payload_content_type"), "payload_content_type")
        _required_receipt_text_allow_empty(
            self.raw.get("host_attestation_base64"),
            "host_attestation_base64",
        )
        if self.usage is None:
            raise _invalid_runtime("runtime receipt summary is missing usage")
        _required_receipt_agent_binding(self.caller_binding, "caller_binding")
        _required_receipt_agent_binding(self.callee_binding, "callee_binding")
        _required_receipt_subject_binding(self.subject_binding, "subject_binding")
        _runtime_receipt_base64(
            self.invocation_nonce_base64,
            "invocation_nonce_base64",
            expected_length=16,
        )
        _required_receipt_text(self.causal_binding_kind, "causal_binding_kind")
        if self.causal_binding is None:
            raise _invalid_runtime("runtime receipt summary is missing causal_binding")
        _validate_runtime_receipt_causal_binding(
            self.causal_binding_kind,
            self.causal_binding,
        )
        _required_receipt_signature(self.callee_signature, "callee_signature")
        assert self.callee_signature is not None
        _runtime_receipt_base64(
            self.callee_signature.signature_base64,
            "callee_signature.signature_base64",
        )
        _required_receipt_agent_binding(self.signer_binding, "signer_binding")
        _validate_runtime_receipt_signing_model(self)
        _required_receipt_text(self.authority_binding_kind, "authority_binding_kind")
        if self.authority_binding is None:
            raise _invalid_runtime(
                "runtime receipt summary is missing authority_binding"
            )
        _required_receipt_text(self.ability_binding, "ability_binding")
        if self.subject_ref is None:
            raise _invalid_runtime("runtime receipt summary is missing subject_ref")
        if self.authority_proof is None:
            raise _invalid_runtime("runtime receipt summary is missing authority_proof")
        if "parent_receipts" not in self.raw:
            raise _invalid_runtime("runtime receipt summary is missing parent_receipts")
        _validate_runtime_receipt_raw_proof_shape(self.raw)
        _validate_runtime_receipt_canonical_proof_facts(self)

    def to_json_dict(self) -> dict[str, object]:
        return _mutable_runtime_receipt_projection(self.raw)


def _immutable_runtime_receipt_projection(
    value: object,
    field_name: str,
) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        raise _invalid_runtime(f"{field_name} must be an object")
    projected: dict[str, object] = {}
    for key, item in value.items():
        if not isinstance(key, str):
            raise _invalid_runtime(f"{field_name} object keys must be strings")
        projected[key] = _immutable_runtime_receipt_value(
            item,
            f"{field_name}.{key}",
        )
    return MappingProxyType(projected)


def _immutable_runtime_receipt_value(value: object, field_name: str) -> object:
    if isinstance(value, Mapping):
        return _immutable_runtime_receipt_projection(value, field_name)
    if isinstance(value, (list, tuple)):
        return tuple(
            _immutable_runtime_receipt_value(item, f"{field_name}[{index}]")
            for index, item in enumerate(value)
        )
    return value


def _mutable_runtime_receipt_projection(value: object) -> dict[str, object]:
    if not isinstance(value, Mapping):
        raise _invalid_runtime("runtime receipt projection must be an object")
    return {
        str(key): _mutable_runtime_receipt_value(item)
        for key, item in value.items()
    }


def _mutable_runtime_receipt_value(value: object) -> object:
    if isinstance(value, Mapping):
        return _mutable_runtime_receipt_projection(value)
    if isinstance(value, (list, tuple)):
        return [_mutable_runtime_receipt_value(item) for item in value]
    return value


@dataclass(frozen=True)
class InvocationResult:
    """Unary invocation terminal result projection."""

    ok: bool
    tuple: InvocationDraft
    terminal_state: str
    output_content_type: str = ""
    output_base64: str = ""
    output_json: Any = None
    elapsed_ms: int = 0
    error: Optional[InvocationFailure] = None
    admission_receipt: Optional[Mapping[str, object]] = None
    admission_receipt_summary: Optional[RuntimeReceipt] = None
    terminal_receipt: Optional[Mapping[str, object]] = None
    terminal_receipt_summary: Optional[RuntimeReceipt] = None

    def __post_init__(self) -> None:
        if not isinstance(self.tuple, InvocationDraft):
            raise _invalid_runtime("invocation result tuple must be an InvocationDraft")
        _required_text(self.terminal_state, "terminal_state")
        if self.elapsed_ms < 0:
            raise _invalid_runtime("elapsed_ms must be non-negative")
        if self.ok and self.error is not None:
            raise _invalid_runtime("ok result must not include error")
        if not self.ok and self.error is None:
            raise _invalid_runtime("failed result must include error")
        if (self.admission_receipt is None) != (self.admission_receipt_summary is None):
            raise _invalid_runtime(
                "admission_receipt and admission_receipt_summary must be projected together"
            )
        if (self.terminal_receipt is None) != (self.terminal_receipt_summary is None):
            raise _invalid_runtime(
                "terminal_receipt and terminal_receipt_summary must be projected together"
            )
        admission = _validated_result_receipt_projection(
            self.admission_receipt,
            self.admission_receipt_summary,
            "admission_receipt",
        )
        terminal = _validated_result_receipt_projection(
            self.terminal_receipt,
            self.terminal_receipt_summary,
            "terminal_receipt",
        )
        _validate_invocation_result_receipt_topology(
            ok=self.ok,
            terminal_state=self.terminal_state,
            failure=self.error,
            admission=admission,
            terminal=terminal,
        )

    @property
    def lifecycle_state(self) -> InvocationLifecycleState:
        """Return the fail-closed canonical terminal lifecycle state."""

        try:
            state = InvocationLifecycleState.from_wire_name(self.terminal_state)
        except ValueError as error:
            raise _invalid_runtime(
                str(error),
                error,
                details={"reason": "invalid_lifecycle_state"},
            ) from error
        if not state.is_terminal:
            raise _invalid_runtime(
                "invocation result lifecycle state must be terminal",
                details={"reason": "invalid_lifecycle_state"},
            )
        return state

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
        reject_retired_top_level_receipt_alias(
            decoded, "invocation result", stage="runtime"
        )
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
            if admission_receipt is not None
            else None
        )
        terminal_receipt_summary = (
            RuntimeReceipt.from_mapping(terminal_receipt)
            if terminal_receipt is not None
            else None
        )
        invocation_id = (
            _optional_string(decoded.get("invocation_id"), "invocation_id") or ""
        )
        output_content_type = (
            _optional_string(decoded.get("output_content_type"), "output_content_type")
            or ""
        )
        output_base64 = (
            _optional_string(decoded.get("output_base64"), "output_base64") or ""
        )
        if (
            invocation_id
            and terminal_receipt_summary is not None
            and invocation_id != terminal_receipt_summary.invocation_id
        ):
            raise _invalid_runtime(
                "invocation result id does not match canonical receipt checkpoints"
            )
        return cls(
            ok=ok,
            tuple=draft,
            terminal_state=terminal_state,
            output_content_type=output_content_type,
            output_base64=output_base64,
            output_json=_normalized_invocation_output_json(
                decoded.get("output_json"),
                output_base64,
                output_content_type,
            ),
            elapsed_ms=elapsed_ms,
            error=failure,
            admission_receipt=admission_receipt,
            admission_receipt_summary=admission_receipt_summary,
            terminal_receipt=terminal_receipt,
            terminal_receipt_summary=terminal_receipt_summary,
        )


def _normalized_invocation_output_json(
    raw: object,
    output_base64: str,
    output_content_type: str,
) -> object:
    if raw is not None:
        return raw
    if "json" not in output_content_type.lower() or not output_base64.strip():
        return raw
    try:
        payload = base64.b64decode(output_base64.strip(), validate=True)
    except Exception as exc:
        raise _invalid_runtime(
            "output_base64 must be valid base64 for JSON invocation output",
            exc,
        ) from exc
    try:
        return json.loads(payload.decode("utf-8"))
    except Exception as exc:
        raise _invalid_runtime(
            "output_base64 must decode to valid JSON for JSON invocation output",
            exc,
        ) from exc


_PRE_ADMISSION_FAILURE_STAGES = frozenset(
    {
        "global_admission",
        "caller_authentication",
        "authority_validation",
        "bootstrap_authorization",
        "quota",
        "ability_resolution",
        "ability_policy",
        "request_validation",
    }
)
_TERMINAL_RECEIPT_STATES = frozenset(
    {
        InvocationLifecycleState.COMPLETED,
        InvocationLifecycleState.FAILED,
        InvocationLifecycleState.TIMED_OUT,
        InvocationLifecycleState.CANCELLED,
    }
)


def _validated_result_receipt_projection(
    raw: Optional[Mapping[str, object]],
    summary: Optional[RuntimeReceipt],
    field_name: str,
) -> Optional[RuntimeReceipt]:
    if raw is None:
        return None
    if summary is None:
        raise _invalid_runtime(f"{field_name} summary is required")
    canonical = RuntimeReceipt.from_mapping(raw)
    if canonical != summary:
        raise _invalid_runtime(
            f"{field_name} summary does not match its raw projection"
        )
    return canonical


def _validate_invocation_result_receipt_topology(
    *,
    ok: bool,
    terminal_state: str,
    failure: Optional[InvocationFailure],
    admission: Optional[RuntimeReceipt],
    terminal: Optional[RuntimeReceipt],
) -> None:
    if (admission is None) != (terminal is None):
        raise _invalid_runtime(
            "invocation result must carry both admission_receipt and "
            "terminal_receipt or neither"
        )
    try:
        result_terminal_state = InvocationLifecycleState.from_wire_name(terminal_state)
    except ValueError as error:
        raise _invalid_runtime(
            str(error),
            error,
            details={"reason": "invalid_lifecycle_state"},
        ) from error
    if admission is None:
        if (
            ok
            or result_terminal_state is not InvocationLifecycleState.FAILED
            or failure is None
            or failure.stage not in _PRE_ADMISSION_FAILURE_STAGES
        ):
            raise _invalid_runtime(
                "receipt-free invocation result requires a typed pre-admission "
                "Failed outcome"
            )
        return

    assert terminal is not None
    admission_state = admission.lifecycle_state
    if admission_state is not InvocationLifecycleState.ADMITTED:
        raise _invalid_runtime(
            "admission_receipt does not carry a canonical admission state"
        )
    if admission.receipt_type != _canonical_receipt_type(admission_state):
        raise _invalid_runtime(
            "admission_receipt does not carry canonical receipt_type admitted"
        )
    if admission.cleanup_complete is not False:
        raise _invalid_runtime("admission_receipt cleanup_complete must be false")
    terminal_receipt_state = terminal.lifecycle_state
    if terminal_receipt_state not in _TERMINAL_RECEIPT_STATES:
        raise _invalid_runtime(
            "terminal_receipt does not carry a canonical terminal state"
        )
    if terminal.receipt_type != _canonical_receipt_type(terminal_receipt_state):
        raise _invalid_runtime(
            "terminal_receipt receipt_type does not match its terminal state"
        )
    if terminal_receipt_state is not result_terminal_state:
        raise _invalid_runtime(
            "terminal_receipt state does not match invocation terminal_state"
        )
    if ok != (terminal_receipt_state is InvocationLifecycleState.COMPLETED):
        raise _invalid_runtime(
            "invocation result ok flag does not match terminal receipt state"
        )
    if terminal.cleanup_complete is not True:
        raise _invalid_runtime("terminal_receipt cleanup_complete must be true")
    if terminal.index <= admission.index:
        raise _invalid_runtime(
            "terminal_receipt index must follow admission_receipt index"
        )
    if admission.invocation_id != terminal.invocation_id:
        raise _invalid_runtime(
            "admission_receipt and terminal_receipt bind different invocations"
        )
    if terminal.timestamp_unix_ms < admission.timestamp_unix_ms:
        raise _invalid_runtime("terminal_receipt timestamp precedes admission_receipt")
    binding_pairs = (
        (admission.caller_binding, terminal.caller_binding),
        (admission.callee_binding, terminal.callee_binding),
        (admission.subject_binding, terminal.subject_binding),
        (admission.invocation_nonce_base64, terminal.invocation_nonce_base64),
        (admission.causal_binding_kind, terminal.causal_binding_kind),
        (admission.causal_binding, terminal.causal_binding),
        (admission.signer_binding, terminal.signer_binding),
        (admission.host_attestation_base64, terminal.host_attestation_base64),
        (admission.authority_binding_kind, terminal.authority_binding_kind),
        (admission.authority_binding, terminal.authority_binding),
        (admission.ability_binding, terminal.ability_binding),
        (admission.subject_ref, terminal.subject_ref),
        (admission.descriptor_version, terminal.descriptor_version),
        (admission.schema_hash_hex, terminal.schema_hash_hex),
        (admission.impl_hash_hex, terminal.impl_hash_hex),
        (admission.runtime_env, terminal.runtime_env),
        (admission.authority_proof, terminal.authority_proof),
        (admission.input_hash_hex, terminal.input_hash_hex),
        (admission.parent_receipts, terminal.parent_receipts),
    )
    if any(left != right for left, right in binding_pairs):
        raise _invalid_runtime(
            "canonical receipt checkpoints contain conflicting invocation bindings"
        )


@dataclass(frozen=True)
class InvocationCancel:
    """Runtime cancellation outcome for a submitted handle."""

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
        raise _invalid_runtime(f"decode invocation cancel JSON: {exc}", exc) from exc
    if not isinstance(decoded, dict):
        raise _invalid_runtime("invocation cancel JSON must be an object")
    handle_id = _required_positive_int(decoded, "handle_id")
    if expected_control is not None:
        if expected_control._adapter_handle_id() != handle_id:
            raise _invalid_runtime(
                "handle_id does not match invocation control capability"
            )
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
        call_mode: str,
        caller_ura: str = "",
        subject_ura: str = "",
        provider: str = "",
    ) -> str:
        """Resolve a runtime-governed AbilityDescriptorRef through the provider."""

        transport = self._require_open()
        if not isinstance(transport, DescriptorResolverTransport):
            raise _invalid_runtime_client(
                "runtime transport does not expose descriptor resolution"
            )
        request = _admitted_descriptor_ref_request(
            callee_ura=callee_ura,
            ability=ability,
            call_mode=call_mode,
            caller_ura=caller_ura,
            subject_ura=subject_ura,
            provider=provider,
        )
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
            raise _transport_error(
                "resolve descriptor_ref transport failed", exc
            ) from exc
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

    def open_signed_stream(self, signed: SignedInvocation) -> StreamHandle:
        transport = self._require_open()
        if not signed.submit_ready():
            raise _invalid_runtime("signed invocation is not submit-ready")
        try:
            stream_transport, open_json = transport.open_stream(
                signed.to_json().encode("utf-8")
            )
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("open signed stream transport failed", exc) from exc
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

    def open_signed_bidi(
        self,
        signed: SignedInvocation,
        streams: tuple[BidiStreamDescriptor, ...],
    ) -> BidiSession:
        transport = self._require_open()
        if not signed.submit_ready():
            raise _invalid_runtime("signed invocation is not submit-ready")
        try:
            streams_json = json.dumps(
                [stream.to_json_dict() for stream in streams],
                separators=(",", ":"),
                sort_keys=True,
            ).encode("utf-8")
            bidi_transport, open_json = transport.open_bidi(
                signed.to_json().encode("utf-8"),
                streams_json,
            )
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("open signed bidi transport failed", exc) from exc
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

    def recover(self, request: RuntimeRecoveryRequest) -> RuntimeRecoveryReport:
        transport = self._require_open()
        if not isinstance(transport, RuntimeRecoveryTransport):
            raise _invalid_runtime_client(
                "runtime transport does not expose restart recovery"
            )
        try:
            raw = transport.recover(request.to_json_bytes())
        except SDKError:
            raise
        except Exception as exc:
            raise _transport_error("runtime recovery transport failed", exc) from exc
        return RuntimeRecoveryReport.from_json(raw)

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
        return InvocationHandle._from_json_with_control(raw, control)._bind_runtime(
            self
        )

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


def _runtime_recovery_event(value: object) -> RuntimeRecoveryEvent:
    if not isinstance(value, dict):
        raise _invalid_runtime("recovery event must be an object")
    return RuntimeRecoveryEvent(
        sequence=_required_positive_int(value, "sequence"),
        kind=_required_string(value, "kind"),
        terminal=_required_bool(value, "terminal"),
        invocation_id=_optional_string(value.get("invocation_id"), "invocation_id")
        or "",
        state=_optional_string(value.get("state"), "state") or "",
        receipt_ura=_optional_string(value.get("receipt_ura"), "receipt_ura") or "",
        reason=_optional_string(value.get("reason"), "reason") or "",
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


def _required_non_negative_int(value: object, field_name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise _invalid_runtime(f"{field_name} is required")
    if value < 0:
        raise _invalid_runtime(f"{field_name} must be a non-negative integer")
    return value


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_runtime(f"{field_name} is required")
    return value


def _admitted_descriptor_ref_request(
    *,
    callee_ura: str,
    ability: str,
    call_mode: str,
    caller_ura: str,
    subject_ura: str,
    provider: str,
) -> dict[str, object]:
    callee_ura = _required_runtime_client_text(callee_ura, "callee_ura")
    ability = _required_runtime_client_text(ability, "ability")
    call_mode = _required_runtime_client_text(call_mode, "call_mode")
    caller_ura = _optional_runtime_client_text(caller_ura)
    subject_ura = _optional_runtime_client_text(subject_ura)
    provider = _optional_runtime_client_text(provider)

    if provider:
        for field_name, value in (
            ("caller_ura", caller_ura),
            ("subject_ura", subject_ura),
        ):
            if not value:
                raise _invalid_runtime_client(
                    f"descriptor_ref provider request requires {field_name}"
                )
            if contains_all_zero_principal(value):
                raise _invalid_runtime_client(
                    f"descriptor_ref provider request {field_name} must not be all-zero"
                )

    request: dict[str, object] = {
        "callee_ura": callee_ura,
        "ability": ability,
        "call_mode": call_mode,
    }
    if caller_ura:
        request["caller_ura"] = caller_ura
    if subject_ura:
        request["subject_ura"] = subject_ura
    if provider:
        request["provider"] = provider
    return request


def _required_runtime_client_text(value: object, field_name: str) -> str:
    if not isinstance(value, str) or value.strip() == "":
        if field_name == "call_mode":
            raise _invalid_runtime_client("descriptor_ref call_mode is required")
        raise _invalid_runtime_client(f"descriptor_ref {field_name} is required")
    return value.strip()


def _optional_runtime_client_text(value: object) -> str:
    if value is None:
        return ""
    return str(value).strip()


def _required_text(value: object, field_name: str) -> str:
    if not isinstance(value, str) or value.strip() == "":
        raise _invalid_runtime(f"{field_name} is required")
    return value.strip()


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


def _decode_runtime_receipt_mapping(
    decoded: Mapping[str, object],
) -> _DecodedRuntimeReceipt:
    if not isinstance(decoded, Mapping):
        raise _invalid_runtime("runtime receipt summary must be an object")
    return _DecodedRuntimeReceipt(
        receipt_id=_optional_string(decoded.get("receipt_id"), "receipt_id") or "",
        receipt_ura=_optional_string(decoded.get("receipt_ura"), "receipt_ura") or "",
        invocation_id=_optional_string(decoded.get("invocation_id"), "invocation_id")
        or "",
        receipt_type=_optional_runtime_summary_text(
            decoded.get("receipt_type"), "receipt_type"
        )
        or "",
        state=_optional_string(decoded.get("state"), "state") or "",
        index=_optional_non_negative_int(decoded.get("index"), "index"),
        timestamp_unix_ms=_optional_non_negative_int(
            decoded.get("timestamp_unix_ms"), "timestamp_unix_ms"
        ),
        prev_receipt_hash_hex=_optional_string(
            decoded.get("prev_receipt_hash_hex"), "prev_receipt_hash_hex"
        )
        or "",
        self_hash_hex=_optional_string(decoded.get("self_hash_hex"), "self_hash_hex")
        or "",
        cleanup_complete=_optional_bool(
            decoded.get("cleanup_complete"), "cleanup_complete"
        ),
        reason=_optional_string(decoded.get("reason"), "reason") or "",
        child_invocation_id=_optional_string(
            decoded.get("child_invocation_id"), "child_invocation_id"
        )
        or "",
        payload_base64=_optional_string(decoded.get("payload_base64"), "payload_base64")
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
        impl_hash_hex=_optional_string(decoded.get("impl_hash_hex"), "impl_hash_hex")
        or "",
        runtime_env=_optional_string(decoded.get("runtime_env"), "runtime_env") or "",
        authority_proof=_receipt_authority_proof(
            decoded.get("authority_proof"), "authority_proof"
        ),
        input_hash_hex=_optional_string(decoded.get("input_hash_hex"), "input_hash_hex")
        or "",
        output_hash_hex=_optional_string(
            decoded.get("output_hash_hex"), "output_hash_hex"
        )
        or "",
        parent_receipts=_receipt_refs(decoded.get("parent_receipts")),
    )


def _runtime_receipt_hash(
    value: object,
    field_name: str,
    *,
    allow_zero: bool = False,
) -> bytes:
    if not isinstance(value, str) or not value:
        raise _invalid_runtime(f"{field_name} is required")
    try:
        decoded = bytes.fromhex(value)
    except ValueError as error:
        raise _invalid_runtime(f"{field_name} must be hexadecimal", error) from error
    if len(decoded) != 32:
        raise _invalid_runtime(f"{field_name} must be exactly 32 bytes")
    if not allow_zero and not any(decoded):
        raise _invalid_runtime(f"{field_name} must not be all-zero")
    return decoded


def _runtime_receipt_base64(
    value: object,
    field_name: str,
    *,
    expected_length: Optional[int] = None,
    allow_empty: bool = False,
) -> bytes:
    if not isinstance(value, str):
        raise _invalid_runtime(f"{field_name} must be a base64 string")
    text = value.strip()
    if not text and not allow_empty:
        raise _invalid_runtime(f"{field_name} is required")
    try:
        decoded = base64.b64decode(text, validate=True)
    except (ValueError, TypeError) as error:
        raise _invalid_runtime(f"{field_name} must be valid base64", error) from error
    if not decoded and not allow_empty:
        raise _invalid_runtime(f"{field_name} must decode to non-empty bytes")
    if expected_length is not None and len(decoded) != expected_length:
        raise _invalid_runtime(
            f"{field_name} must decode to exactly {expected_length} bytes"
        )
    return decoded


def _canonical_receipt_type(state: InvocationLifecycleState) -> str:
    receipt_types = {
        InvocationLifecycleState.ACCEPTED: "accepted",
        InvocationLifecycleState.ADMITTED: "admitted",
        InvocationLifecycleState.DISPATCHED: "dispatched",
        InvocationLifecycleState.RUNNING: "running",
        InvocationLifecycleState.COMPLETED: "completed",
        InvocationLifecycleState.FAILED: "failed",
        InvocationLifecycleState.TIMED_OUT: "timed_out",
        InvocationLifecycleState.CANCELLED: "cancelled",
    }
    return receipt_types.get(state, "")


def _validate_runtime_receipt_ref(
    value: object,
    field_name: str,
) -> None:
    decoded = _receipt_mapping(value, field_name)
    if decoded is None:
        raise _invalid_runtime(f"{field_name} must be an object")
    _require_runtime_receipt_exact_keys(
        decoded,
        field_name,
        "receipt_hash_hex",
        "receipt_ura",
    )
    _runtime_receipt_hash(
        decoded.get("receipt_hash_hex"),
        f"{field_name}.receipt_hash_hex",
    )
    _required_receipt_text(
        decoded.get("receipt_ura"),
        f"{field_name}.receipt_ura",
    )


def _validate_runtime_receipt_causal_binding(
    binding_kind: str,
    binding: Mapping[str, object],
) -> None:
    form = _required_receipt_text(binding.get("form"), "causal_binding.form")
    if form != binding_kind:
        raise _invalid_runtime(
            "runtime receipt causal_binding form does not match causal_binding_kind"
        )
    if form == "none":
        _require_runtime_receipt_exact_keys(binding, "causal_binding", "form")
        return
    if form == "scalar":
        _require_runtime_receipt_exact_keys(
            binding,
            "causal_binding",
            "form",
            "receipt",
        )
        _validate_runtime_receipt_ref(
            binding.get("receipt"),
            "causal_binding.receipt",
        )
        return
    if form == "list":
        _require_runtime_receipt_exact_keys(
            binding,
            "causal_binding",
            "form",
            "prior",
        )
        prior = binding.get("prior")
        prior_items = _runtime_receipt_raw_array(
            prior,
            "causal_binding.prior",
            require_non_empty=True,
        )
        for index, receipt in enumerate(prior_items):
            _validate_runtime_receipt_ref(
                receipt,
                f"causal_binding.prior[{index}]",
        )
        return
    if form == "merkle":
        _require_runtime_receipt_exact_keys(
            binding,
            "causal_binding",
            "form",
            "root_hex",
            "proof_ura",
        )
        _runtime_receipt_hash(
            binding.get("root_hex"),
            "causal_binding.root_hex",
        )
        _required_receipt_text(
            binding.get("proof_ura"),
            "causal_binding.proof_ura",
        )
        return
    raise _invalid_runtime(f"unsupported causal_binding form {form!r}")


def _validate_runtime_receipt_signing_model(receipt: RuntimeReceipt) -> None:
    assert receipt.callee_binding is not None
    assert receipt.signer_binding is not None
    signer_ura = receipt.signer_binding.ura.strip()
    callee_ura = receipt.callee_binding.ura.strip()
    if signer_ura == callee_ura:
        if receipt.host_attestation_base64.strip():
            raise _invalid_runtime(
                "self-signed runtime receipt must not carry host_attestation_base64"
            )
        return
    if not receipt.host_attestation_base64.strip():
        raise _invalid_runtime(
            "hosted runtime receipt is missing host_attestation_base64"
        )
    _runtime_receipt_base64(
        receipt.host_attestation_base64,
        "host_attestation_base64",
        expected_length=64,
    )


def _validate_runtime_receipt_raw_proof_shape(raw: Mapping[str, object]) -> None:
    for field_name in (
        "caller_binding",
        "callee_binding",
        "subject_binding",
        "signer_binding",
    ):
        _require_runtime_receipt_exact_keys(
            _runtime_receipt_raw_mapping(raw.get(field_name), field_name),
            field_name,
            "ura",
            "profile",
        )

    _require_runtime_receipt_exact_keys(
        _runtime_receipt_raw_mapping(raw.get("subject_ref"), "subject_ref"),
        "subject_ref",
        "kind",
        "ura",
        "profile",
    )
    _require_runtime_receipt_exact_keys(
        _runtime_receipt_raw_mapping(raw.get("callee_signature"), "callee_signature"),
        "callee_signature",
        "algorithm",
        "signature_base64",
        "key_id_hint",
    )

    authority = _runtime_receipt_raw_mapping(
        raw.get("authority_binding"),
        "authority_binding",
    )
    _validate_runtime_receipt_authority_binding_shape(
        authority,
        "authority_binding",
    )

    proof = _runtime_receipt_raw_mapping(raw.get("authority_proof"), "authority_proof")
    _require_runtime_receipt_exact_keys(
        proof,
        "authority_proof",
        "proof_type",
        "binding_kind",
        "binding",
        "proof_payload_base64",
        "proof_hash_hex",
        "issuer",
        "signature",
        "admission_hook",
    )
    if "proof_payload_base64" not in proof:
        raise _invalid_runtime(
            "runtime receipt summary is missing authority_proof.proof_payload_base64"
        )
    _validate_runtime_receipt_authority_binding_shape(
        _runtime_receipt_raw_mapping(
            proof.get("binding"),
            "authority_proof.binding",
        ),
        "authority_proof.binding",
    )
    _require_runtime_receipt_exact_keys(
        _runtime_receipt_raw_mapping(
            proof.get("issuer"),
            "authority_proof.issuer",
        ),
        "authority_proof.issuer",
        "ura",
        "profile",
    )
    _require_runtime_receipt_exact_keys(
        _runtime_receipt_raw_mapping(
            proof.get("signature"),
            "authority_proof.signature",
        ),
        "authority_proof.signature",
        "algorithm",
        "signature_base64",
        "key_id_hint",
    )

    parents = raw.get("parent_receipts")
    for index, parent in enumerate(
        _runtime_receipt_raw_array(parents, "parent_receipts")
    ):
        field_name = f"parent_receipts[{index}]"
        _require_runtime_receipt_exact_keys(
            _runtime_receipt_raw_mapping(parent, field_name),
            field_name,
            "receipt_hash_hex",
            "receipt_ura",
        )


def _validate_runtime_receipt_authority_binding_shape(
    value: Mapping[str, object],
    field_name: str,
) -> None:
    kind = _required_receipt_text(value.get("kind"), f"{field_name}.kind")
    if kind == "self":
        _require_runtime_receipt_exact_keys(value, field_name, "kind", "principal_ura")
        return
    if kind == "delegation":
        _require_runtime_receipt_exact_keys(
            value,
            field_name,
            "kind",
            "issuer_ura",
            "subject_ura",
            "caller_ura",
            "audience",
            "scopes",
            "issued_at_ms",
            "expires_at_ms",
            "signature_base64",
        )
        return
    if kind == "capability":
        _require_runtime_receipt_exact_keys(value, field_name, "kind", "capability_ura")
        return
    if kind == "policy":
        _require_runtime_receipt_exact_keys(value, field_name, "kind", "policy_ura")
        return
    if kind == "session":
        _require_runtime_receipt_exact_keys(
            value,
            field_name,
            "kind",
            "issuer_ura",
            "subject_ura",
            "session_id",
            "scopes",
            "audiences",
            "issued_at_ms",
            "expires_at_ms",
            "signature_base64",
        )
        return
    if kind == "bootstrap":
        _require_runtime_receipt_exact_keys(
            value,
            field_name,
            "kind",
            "principal_ura",
            "realm",
            "ability",
        )
        return
    raise _invalid_runtime(f"{field_name}.kind is not canonical: {kind!r}")


def _runtime_receipt_raw_mapping(
    value: object,
    field_name: str,
) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        raise _invalid_runtime(f"{field_name} must be an object")
    return value


def _runtime_receipt_raw_array(
    value: object,
    field_name: str,
    *,
    require_non_empty: bool = False,
) -> tuple[object, ...]:
    if not isinstance(value, (list, tuple)):
        raise _invalid_runtime(f"{field_name} must be an array")
    if require_non_empty and not value:
        raise _invalid_runtime(f"{field_name} must be a non-empty array")
    return tuple(value)


def _require_runtime_receipt_exact_keys(
    value: Mapping[str, object],
    field_name: str,
    *allowed_keys: str,
) -> None:
    allowed = set(allowed_keys)
    for key in value:
        if key not in allowed:
            raise _invalid_runtime(f"{field_name} contains noncanonical field {key}")


def _require_runtime_receipt_required_keys(
    value: Mapping[str, object],
    field_name: str,
    *required_keys: str,
) -> None:
    for key in required_keys:
        if key not in value:
            raise _invalid_runtime(
                f"runtime receipt summary is missing {field_name}.{key}"
            )


def _validate_runtime_receipt_canonical_proof_facts(
    receipt: RuntimeReceipt,
) -> None:
    assert receipt.caller_binding is not None
    assert receipt.callee_binding is not None
    assert receipt.subject_binding is not None
    assert receipt.signer_binding is not None
    assert receipt.authority_binding is not None
    assert receipt.subject_ref is not None
    assert receipt.authority_proof is not None

    for field_name, binding in (
        ("caller_binding", receipt.caller_binding),
        ("callee_binding", receipt.callee_binding),
        ("subject_binding", receipt.subject_binding),
        ("signer_binding", receipt.signer_binding),
    ):
        _runtime_receipt_ura_profile(binding.profile, f"{field_name}.profile")

    authority = _runtime_receipt_authority_binding(
        receipt.authority_binding,
        "authority_binding",
    )
    authority_kind = _required_receipt_text(
        receipt.authority_binding.get("kind"),
        "authority_binding.kind",
    )
    if authority_kind != receipt.authority_binding_kind:
        raise _invalid_runtime(
            "runtime receipt authority_binding kind does not match "
            "authority_binding_kind"
        )

    proof = receipt.authority_proof
    _required_receipt_text(proof.proof_type, "authority_proof.proof_type")
    proof_kind = _required_receipt_text(
        proof.binding_kind,
        "authority_proof.binding_kind",
    )
    if proof_kind != receipt.authority_binding_kind:
        raise _invalid_runtime(
            "runtime receipt authority_proof binding_kind does not match "
            "authority_binding_kind"
        )
    proof_binding = _runtime_receipt_authority_binding(
        proof.binding,
        "authority_proof.binding",
    )
    if proof_binding != authority:
        raise _invalid_runtime(
            "runtime receipt authority_proof binding does not match authority_binding"
        )

    _required_receipt_agent_binding(proof.issuer, "authority_proof.issuer")
    assert proof.issuer is not None
    issuer_profile = _runtime_receipt_ura_profile(
        proof.issuer.profile,
        "authority_proof.issuer.profile",
    )
    callee_profile = _runtime_receipt_ura_profile(
        receipt.callee_binding.profile,
        "callee_binding.profile",
    )
    issuer = _AxonAgentIdentity(proof.issuer.ura, issuer_profile)
    callee = _AxonAgentIdentity(receipt.callee_binding.ura, callee_profile)
    if issuer != callee:
        raise _invalid_runtime(
            "runtime receipt authority_proof issuer does not match callee_binding"
        )

    _required_receipt_signature(
        proof.signature,
        "authority_proof.signature",
    )
    assert proof.signature is not None
    proof_signature = _AxonCalleeSignature(
        proof.signature.algorithm,
        _runtime_receipt_base64(
            proof.signature.signature_base64,
            "authority_proof.signature.signature_base64",
        ),
        proof.signature.key_id_hint,
    )

    subject_kind = {
        1: _AxonEntityRefKind.RESOURCE,
        2: _AxonEntityRefKind.AGENT,
        3: _AxonEntityRefKind.ABILITY,
        4: _AxonEntityRefKind.SESSION,
        5: _AxonEntityRefKind.CONTINUATION,
        6: _AxonEntityRefKind.STATE_OBJECT,
        7: _AxonEntityRefKind.DEVICE,
    }.get(receipt.subject_ref.kind)
    if subject_kind is None:
        raise _invalid_runtime("subject_ref.kind is not a canonical EntityRef kind")
    subject_ref = _AxonEntityRef(
        kind=subject_kind,
        ura=_required_receipt_text(receipt.subject_ref.ura, "subject_ref.ura"),
        profile=_runtime_receipt_ura_profile(
            receipt.subject_ref.profile,
            "subject_ref.profile",
        ),
    )

    parent_receipts = tuple(
        _AxonReceiptRef(
            receipt_hash=_runtime_receipt_hash(
                parent.receipt_hash_hex,
                f"parent_receipts[{index}].receipt_hash_hex",
            ),
            receipt_ura=_required_receipt_text(
                parent.receipt_ura,
                f"parent_receipts[{index}].receipt_ura",
            ),
        )
        for index, parent in enumerate(receipt.parent_receipts)
    )
    try:
        authority_proof = _AxonInvocationAuthorityProof(
            proof_type=proof.proof_type,
            binding=proof_binding,
            proof_payload=_runtime_receipt_base64(
                proof.proof_payload_base64,
                "authority_proof.proof_payload_base64",
                allow_empty=True,
            ),
            proof_hash=_runtime_receipt_hash(
                proof.proof_hash_hex,
                "authority_proof.proof_hash_hex",
            ),
            issuer=issuer,
            signature=proof_signature,
            admission_hook=_required_receipt_text(
                proof.admission_hook,
                "authority_proof.admission_hook",
            ),
        )
        _AxonReceiptProofFacts(
            subject_ref=subject_ref,
            descriptor_version=_required_receipt_text(
                receipt.descriptor_version,
                "descriptor_version",
            ),
            schema_hash=_runtime_receipt_hash(
                receipt.schema_hash_hex,
                "schema_hash_hex",
            ),
            impl_hash=_runtime_receipt_hash(
                receipt.impl_hash_hex,
                "impl_hash_hex",
            ),
            runtime_env=_required_receipt_text(
                receipt.runtime_env,
                "runtime_env",
            ),
            authority_proof=authority_proof,
            input_hash=_runtime_receipt_hash(
                receipt.input_hash_hex,
                "input_hash_hex",
            ),
            output_hash=_runtime_receipt_hash(
                receipt.output_hash_hex,
                "output_hash_hex",
            ),
            parent_receipts=parent_receipts,
        )
    except _AxonError as error:
        raise _invalid_runtime(
            f"runtime receipt proof facts are not canonical: {error}",
            error,
        ) from error


def _runtime_receipt_authority_binding(
    value: Optional[Mapping[str, object]],
    field_name: str,
) -> _AxonAuthorityBinding:
    if value is None:
        raise _invalid_runtime(f"runtime receipt summary is missing {field_name}")
    kind = _required_receipt_text(value.get("kind"), f"{field_name}.kind")
    if kind == "self":
        return _AxonAuthorityBinding.self_(
            _required_receipt_text(
                value.get("principal_ura"),
                f"{field_name}.principal_ura",
            )
        )
    if kind == "delegation":
        return _AxonAuthorityBinding.delegated(
            _AxonDelegationProofBody(
                issuer_ura=_required_receipt_text(
                    value.get("issuer_ura"),
                    f"{field_name}.issuer_ura",
                ),
                subject_ura=_required_receipt_text(
                    value.get("subject_ura"),
                    f"{field_name}.subject_ura",
                ),
                caller_ura=_required_receipt_text(
                    value.get("caller_ura"),
                    f"{field_name}.caller_ura",
                ),
                audience=_required_receipt_text(
                    value.get("audience"),
                    f"{field_name}.audience",
                ),
                scopes=_runtime_receipt_text_tuple(
                    value.get("scopes"),
                    f"{field_name}.scopes",
                ),
                issued_at_ms=_runtime_receipt_required_non_negative_int(
                    value.get("issued_at_ms"),
                    f"{field_name}.issued_at_ms",
                ),
                expires_at_ms=_runtime_receipt_required_non_negative_int(
                    value.get("expires_at_ms"),
                    f"{field_name}.expires_at_ms",
                ),
                signature=_runtime_receipt_base64(
                    value.get("signature_base64"),
                    f"{field_name}.signature_base64",
                    expected_length=64,
                ),
            )
        )
    if kind == "capability":
        return _AxonAuthorityBinding.capability(
            _required_receipt_text(
                value.get("capability_ura"),
                f"{field_name}.capability_ura",
            )
        )
    if kind == "policy":
        return _AxonAuthorityBinding.policy(
            _required_receipt_text(
                value.get("policy_ura"),
                f"{field_name}.policy_ura",
            )
        )
    if kind == "session":
        return _AxonAuthorityBinding.session(
            _AxonSessionAuthorityBody(
                backend_ura=_required_receipt_text(
                    value.get("issuer_ura"),
                    f"{field_name}.issuer_ura",
                ),
                user_ura=_required_receipt_text(
                    value.get("subject_ura"),
                    f"{field_name}.subject_ura",
                ),
                session_id=_required_receipt_text(
                    value.get("session_id"),
                    f"{field_name}.session_id",
                ),
                scopes=_runtime_receipt_text_tuple(
                    value.get("scopes"),
                    f"{field_name}.scopes",
                ),
                audiences=_runtime_receipt_text_tuple(
                    value.get("audiences"),
                    f"{field_name}.audiences",
                ),
                issued_at_ms=_runtime_receipt_required_non_negative_int(
                    value.get("issued_at_ms"),
                    f"{field_name}.issued_at_ms",
                ),
                expires_at_ms=_runtime_receipt_required_non_negative_int(
                    value.get("expires_at_ms"),
                    f"{field_name}.expires_at_ms",
                ),
                signature=_runtime_receipt_base64(
                    value.get("signature_base64"),
                    f"{field_name}.signature_base64",
                    expected_length=64,
                ),
            )
        )
    if kind == "bootstrap":
        return _AxonAuthorityBinding.bootstrap(
            _AxonBootstrapAuthorityBody(
                principal_ura=_required_receipt_text(
                    value.get("principal_ura"),
                    f"{field_name}.principal_ura",
                ),
                realm=_required_receipt_text(
                    value.get("realm"),
                    f"{field_name}.realm",
                ),
                ability=_required_receipt_text(
                    value.get("ability"),
                    f"{field_name}.ability",
                ),
            )
        )
    raise _invalid_runtime(f"{field_name}.kind is not canonical: {kind!r}")


def _runtime_receipt_ura_profile(value: object, field_name: str) -> _AxonUraProfile:
    text = _required_receipt_text(value, field_name)
    try:
        return _AxonUraProfile.parse(text)
    except _AxonError as error:
        raise _invalid_runtime(
            f"{field_name} is not canonical: {error}", error
        ) from error


def _runtime_receipt_text_tuple(
    value: object,
    field_name: str,
) -> tuple[str, ...]:
    if not isinstance(value, (list, tuple)) or not value:
        raise _invalid_runtime(f"{field_name} must be a non-empty array")
    return tuple(
        _required_receipt_text(item, f"{field_name}[{index}]")
        for index, item in enumerate(value)
    )


def _runtime_receipt_required_non_negative_int(
    value: object,
    field_name: str,
) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise _invalid_runtime(f"{field_name} must be a non-negative integer")
    return value


def _required_receipt_text(value: object, field_name: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise _invalid_runtime(f"runtime receipt summary is missing {field_name}")
    return value.strip()


def _required_receipt_text_allow_empty(value: object, field_name: str) -> str:
    if not isinstance(value, str) or value != value.strip():
        raise _invalid_runtime(f"runtime receipt summary is missing {field_name}")
    return value


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


def _required_receipt_agent_binding(
    value: Optional[RuntimeReceiptAgentBinding], field_name: str
) -> None:
    if value is None or not value.ura.strip():
        raise _invalid_runtime(f"runtime receipt summary is missing {field_name}.ura")


def _required_receipt_subject_binding(
    value: Optional[RuntimeReceiptSubjectBinding], field_name: str
) -> None:
    if value is None or not value.ura.strip():
        raise _invalid_runtime(f"runtime receipt summary is missing {field_name}.ura")


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


def _required_receipt_signature(
    value: Optional[RuntimeReceiptSignature], field_name: str
) -> None:
    if value is None or not value.signature_base64.strip():
        raise _invalid_runtime(
            f"runtime receipt summary is missing {field_name}.signature_base64"
        )
    if not value.algorithm.strip():
        raise _invalid_runtime(
            f"runtime receipt summary is missing {field_name}.algorithm"
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
    if not isinstance(value, list):
        raise _invalid_runtime("parent_receipts must be an array")
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
        binding=_optional_mapping(
            decoded.get("binding"),
            f"{field_name}.binding",
        ),
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


def _invalid_runtime(
    message: str,
    cause: Optional[BaseException] = None,
    *,
    details: Optional[Mapping[str, object]] = None,
) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="runtime",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        details=details,
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
