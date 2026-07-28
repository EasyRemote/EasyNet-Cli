"""Product-neutral Receipt facade over Axon-owned audit projections."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Mapping, Protocol, Sequence, cast

from axon_sdk import parse_ura as _parse_ura
from axon_sdk.invocation import (
    ChainCheckResult,
    CausalContext as _AxonCausalContext,
    InvocationCausalLink,
    InvocationLedgerRecord,
    InvocationReceipt,
    InvocationReceiptAnchor,
    InvocationReceiptChainSummary,
    InvocationTraceEdge,
    InvocationTraceGraph,
    KeyResolver,
    ReceiptRef as _AxonReceiptRef,
    VerifiedReceipt,
    causal_to_json as _causal_to_json,
    parse_invocation_ledger_record,
    parse_invocation_trace_graph,
    verify_receipt_chain,
)

from .axon_addressing import parse_ura
from .authority import DelegationProof, SessionAuthority
from .errors import ErrorCode, RetryHint, SDKError
from ._receipt_routes import (
    _RECEIPT_HISTORY_GET,
    _RECEIPT_HISTORY_LIST,
    _RECEIPT_TRACE_GET,
)
from ._receipt_history_admission import validate_receipt_history_request
from ._runtime_subjects import runtime_state_read_subject_ura
from .runtime import RuntimeReceipt
from .runtime_ability import RuntimeAbilityClient, RuntimeCallContext, RuntimeInvocationAuthority

__all__ = [
    "DEFAULT_RECEIPT_PAGE_LIMIT",
    "MAX_RECEIPT_PAGE_LIMIT",
    "ChainCheckResult",
    "InvocationCausalLink",
    "InvocationLedgerRecord",
    "InvocationReceipt",
    "InvocationReceiptAnchor",
    "InvocationReceiptChainSummary",
    "InvocationTraceEdge",
    "InvocationTraceGraph",
    "ReceiptClient",
    "ReceiptFilter",
    "ReceiptGetRequest",
    "ReceiptGetResult",
    "ReceiptHistoryPage",
    "ReceiptHistoryAuthorityScopeProvider",
    "ReceiptLedgerSource",
    "ReceiptListRequest",
    "ReceiptLookup",
    "ReceiptProvider",
    "ReceiptReference",
    "ReceiptTraceRequest",
    "ReceiptTraceResult",
    "RuntimeReceiptProvider",
    "VerifiedReceipt",
    "receipt_read_call_context",
]

DEFAULT_RECEIPT_PAGE_LIMIT = 50
MAX_RECEIPT_PAGE_LIMIT = 500
MAX_RECEIPT_CURSOR_LENGTH = 4096


@dataclass(frozen=True)
class ReceiptLookup:
    """One canonical key for locating an invocation ledger record."""

    invocation_ura: str = ""
    request_id: str = ""
    trace_id: str = ""


@dataclass(frozen=True)
class ReceiptFilter:
    """Shared, provider-backed predicates for history and trace reads."""

    caller_ura: str = ""
    callee_ura: str = ""
    subject_uras: tuple[str, ...] = ()
    ability_uras: tuple[str, ...] = ()
    state: str = ""
    trace_id: str = ""


@dataclass(frozen=True)
class ReceiptListRequest:
    call: RuntimeCallContext
    lookup: ReceiptLookup | None = None
    filter: ReceiptFilter | None = None
    limit: int = 0
    cursor: str = ""
    exclude_ability_uras: tuple[str, ...] = ()


@dataclass(frozen=True)
class ReceiptGetRequest:
    call: RuntimeCallContext
    lookup: ReceiptLookup
    filter: ReceiptFilter | None = None


@dataclass(frozen=True)
class ReceiptTraceRequest:
    call: RuntimeCallContext
    lookup: ReceiptLookup
    filter: ReceiptFilter | None = None


@dataclass(frozen=True)
class ReceiptLedgerSource:
    ledger_ura: str


@dataclass(frozen=True)
class ReceiptHistoryPage:
    source: ReceiptLedgerSource
    records: tuple[InvocationLedgerRecord, ...]
    limit: int
    next_cursor: str = ""


@dataclass(frozen=True)
class ReceiptGetResult:
    source: ReceiptLedgerSource
    record: InvocationLedgerRecord | None


@dataclass(frozen=True)
class ReceiptTraceResult:
    source: ReceiptLedgerSource
    graph: InvocationTraceGraph


@dataclass(frozen=True)
class ReceiptReference:
    """A runtime-issued receipt anchor usable as scalar causality."""

    receipt_ura: str
    receipt_hash: bytes

    def __post_init__(self) -> None:
        receipt_ura = _required_ura(self.receipt_ura, "receipt_ura")
        if not isinstance(self.receipt_hash, bytes):
            raise _invalid("receipt_hash must be bytes")
        try:
            _AxonReceiptRef(
                receipt_hash=self.receipt_hash,
                receipt_ura=receipt_ura,
            )
        except Exception as error:
            raise _invalid("receipt_hash must be exactly 32 bytes", error) from error
        object.__setattr__(self, "receipt_ura", receipt_ura)

    @classmethod
    def from_anchor(cls, anchor: InvocationReceiptAnchor) -> "ReceiptReference":
        """Build a causal reference from an Axon ledger anchor."""

        if not isinstance(anchor, InvocationReceiptAnchor):
            raise _invalid("Invocation receipt anchor is required")
        try:
            receipt_hash = bytes.fromhex(anchor.receipt_hash)
        except (TypeError, ValueError) as error:
            raise _invalid("receipt anchor hash must be hexadecimal", error) from error
        return cls(receipt_ura=anchor.receipt_ura, receipt_hash=receipt_hash)

    @classmethod
    def from_runtime_receipt(cls, receipt: object) -> "ReceiptReference":
        """Build a causal reference from a runtime receipt summary."""

        if isinstance(receipt, RuntimeReceipt):
            receipt.validate_summary()
        receipt_ura = _summary_value(receipt, "receipt_ura")
        self_hash_hex = _summary_value(receipt, "self_hash_hex")
        if not receipt_ura:
            raise _invalid("runtime receipt summary is missing receipt_ura")
        try:
            receipt_hash = bytes.fromhex(self_hash_hex)
        except (TypeError, ValueError) as error:
            raise _invalid(
                "runtime receipt summary self_hash_hex must be hexadecimal",
                error,
            ) from error
        return cls(receipt_ura=receipt_ura, receipt_hash=receipt_hash)

    def causal_context(self) -> Mapping[str, object]:
        """Project this anchor through Axon's canonical scalar JSON codec."""

        try:
            reference = _AxonReceiptRef(
                receipt_hash=self.receipt_hash,
                receipt_ura=self.receipt_ura,
            )
            return cast(Mapping[str, object], _causal_to_json(_AxonCausalContext.scalar(reference)))
        except Exception as error:
            raise _invalid("project scalar receipt causal context", error) from error


class ReceiptProvider(Protocol):
    def list(self, request: ReceiptListRequest) -> ReceiptHistoryPage: ...
    def get(self, request: ReceiptGetRequest) -> ReceiptGetResult: ...
    def trace(self, request: ReceiptTraceRequest) -> ReceiptTraceResult: ...


class ReceiptHistoryAuthorityScopeProvider(Protocol):
    """Provider capability that declares the scope required for history reads."""

    def receipt_history_list_authority_scope(self) -> str: ...


class ReceiptClient:
    """Generic receipt reads plus Axon-owned verification operations."""

    def __init__(self, provider: ReceiptProvider) -> None:
        if provider is None:
            raise _invalid("Receipt provider is required")
        self._provider = provider

    def list(self, request: ReceiptListRequest) -> ReceiptHistoryPage:
        return self._provider.list(request)

    def get(self, request: ReceiptGetRequest) -> ReceiptGetResult:
        return self._provider.get(request)

    def trace(self, request: ReceiptTraceRequest) -> ReceiptTraceResult:
        return self._provider.trace(request)

    def verify(
        self, receipt: InvocationReceipt, resolver: KeyResolver
    ) -> VerifiedReceipt:
        if receipt is None or resolver is None:
            raise _invalid("Invocation receipt and key resolver are required")
        return receipt.verify(resolver)

    def verify_chain(self, receipts: Sequence[InvocationReceipt]) -> ChainCheckResult:
        if isinstance(receipts, (str, bytes, bytearray)):
            raise _invalid("Invocation receipt sequence is required")
        try:
            ordered = list(receipts)
        except TypeError as error:
            raise _invalid("Invocation receipt sequence is required", error) from error
        return verify_receipt_chain(ordered)


def receipt_read_call_context(
    *,
    caller_ura: str,
    callee_ura: str,
    authority: RuntimeInvocationAuthority,
    nonce_base64: str,
    causal_context: Mapping[str, object],
    metadata: Mapping[str, object] | None = None,
) -> RuntimeCallContext:
    """Build the canonical runtime call context for receipt history reads."""

    caller = _required_ura(caller_ura, "caller_ura")
    callee = _required_ura(callee_ura, "callee_ura")
    nonce = _required_text(nonce_base64, "nonce_base64")
    if not isinstance(causal_context, Mapping):
        raise _invalid("causal_context is required")
    return RuntimeCallContext(
        caller_ura=caller,
        callee_ura=callee,
        subject_ura=_receipt_read_subject_ura(callee, authority),
        nonce_base64=nonce,
        causal_context=dict(causal_context),
        metadata=dict(metadata or {}),
        authority=authority,
    )


def _receipt_read_subject_ura(
    callee_ura: str,
    authority: RuntimeInvocationAuthority,
) -> str:
    if isinstance(authority, DelegationProof):
        return _required_ura(authority.subject_ura, "authority.subject_ura")
    if isinstance(authority, SessionAuthority):
        callee = parse_ura(callee_ura)
        return runtime_state_read_subject_ura(
            callee.realm,
            authority.session_owner_user_id,
        )
    raise SDKError(
        code=ErrorCode.AUTHORITY_DENIED,
        stage="receipt",
        retry=RetryHint.NEVER,
        retryable=False,
        message="receipt read call context authority has an unsupported canonical type",
    )


class RuntimeReceiptProvider:
    """Receipt provider composed over the canonical runtime ability kernel."""

    def __init__(self, ability: RuntimeAbilityClient) -> None:
        if ability is None:
            raise _invalid("runtime ability client is required")
        self._ability = ability

    def receipt_history_list_authority_scope(self) -> str:
        return _RECEIPT_HISTORY_LIST

    def list(self, request: ReceiptListRequest) -> ReceiptHistoryPage:
        if not isinstance(request, ReceiptListRequest):
            raise _invalid("Receipt list request is required")
        cursor = _receipt_cursor(request.cursor)
        limit = _receipt_limit(request.limit)
        arguments = _query_arguments(
            request.lookup, request.filter, lookup_required=False
        )
        arguments["limit"] = limit
        if cursor:
            arguments["cursor"] = cursor
        excluded = _ura_sequence(
            request.exclude_ability_uras,
            "exclude_ability_uras",
        )
        if excluded:
            arguments["exclude_ability_uras"] = list(excluded)
        validate_receipt_history_request(
            request.call,
            request.filter,
            self.receipt_history_list_authority_scope(),
        )
        output = self._ability._invoke_governance_read(
            request.call,
            _RECEIPT_HISTORY_LIST,
            arguments,
        )
        records_raw = output.get("records")
        if not isinstance(records_raw, list):
            raise _invalid("Receipt history records must be a list")
        if len(records_raw) > limit:
            raise _invalid(
                "runtime Receipt history exceeds the bounded page and has no stable cursor"
            )
        next_cursor = _optional_output_text(output, "next_cursor")
        if next_cursor and next_cursor == cursor:
            raise _invalid("runtime Receipt history returned a repeated cursor")
        records = tuple(_parse_record(item) for item in records_raw)
        return ReceiptHistoryPage(
            source=_project_source(output),
            records=records,
            limit=limit,
            next_cursor=next_cursor,
        )

    def get(self, request: ReceiptGetRequest) -> ReceiptGetResult:
        if not isinstance(request, ReceiptGetRequest):
            raise _invalid("Receipt get request is required")
        output = self._ability._invoke_governance_read(
            request.call,
            _RECEIPT_HISTORY_GET,
            _query_arguments(request.lookup, request.filter, lookup_required=True),
        )
        if "record" not in output:
            raise _invalid("Receipt get result must include record")
        record_raw = output.get("record")
        record = None if record_raw is None else _parse_record(record_raw)
        return ReceiptGetResult(source=_project_source(output), record=record)

    def trace(self, request: ReceiptTraceRequest) -> ReceiptTraceResult:
        if not isinstance(request, ReceiptTraceRequest):
            raise _invalid("Receipt trace request is required")
        output = self._ability._invoke_governance_read(
            request.call,
            _RECEIPT_TRACE_GET,
            _query_arguments(request.lookup, request.filter, lookup_required=True),
        )
        graph_raw = {
            "trace_id": output.get("trace_id"),
            "records": output.get("nodes"),
            "edges": output.get("edges"),
        }
        try:
            graph = parse_invocation_trace_graph(graph_raw)
        except Exception as error:
            raise _invalid("invalid Axon invocation trace projection", error) from error
        return ReceiptTraceResult(source=_project_source(output), graph=graph)


def _query_arguments(
    lookup: ReceiptLookup | None,
    receipt_filter: ReceiptFilter | None,
    *,
    lookup_required: bool,
) -> dict[str, object]:
    arguments: dict[str, object] = {}
    key = _project_lookup(lookup, required=lookup_required)
    if key:
        arguments["key"] = key
    projected_filter = _project_filter(receipt_filter)
    if projected_filter:
        arguments["filter"] = projected_filter
    return arguments


def _project_lookup(
    lookup: ReceiptLookup | None, *, required: bool
) -> dict[str, object]:
    if lookup is None:
        if required:
            raise _invalid("exactly one Receipt lookup key is required")
        return {}
    if not isinstance(lookup, ReceiptLookup):
        raise _invalid("Receipt lookup is required")
    values = (
        ("ura", _optional_ura(lookup.invocation_ura, "invocation_ura")),
        ("request_id", _optional_text(lookup.request_id, "request_id")),
        ("trace_id", _optional_text(lookup.trace_id, "trace_id")),
    )
    selected = [(name, value) for name, value in values if value]
    if len(selected) != 1:
        raise _invalid("exactly one Receipt lookup key is required")
    name, value = selected[0]
    return {name: value}


def _project_filter(receipt_filter: ReceiptFilter | None) -> dict[str, object]:
    if receipt_filter is None:
        return {}
    if not isinstance(receipt_filter, ReceiptFilter):
        raise _invalid("Receipt filter is required")
    projected: dict[str, object] = {}
    for name in ("caller_ura", "callee_ura"):
        value = _optional_ura(getattr(receipt_filter, name), name)
        if value:
            projected[name] = value
    for name in ("state", "trace_id"):
        value = _optional_text(getattr(receipt_filter, name), name)
        if value:
            projected[name] = value
    subjects = _ura_sequence(receipt_filter.subject_uras, "subject_uras")
    if subjects:
        projected["subject_uras"] = list(subjects)
    abilities = _ura_sequence(receipt_filter.ability_uras, "ability_uras")
    if len(abilities) == 1:
        projected["ability_ura"] = abilities[0]
    elif abilities:
        projected["ability_uras"] = list(abilities)
    return projected


def _project_source(output: Mapping[str, object]) -> ReceiptLedgerSource:
    return ReceiptLedgerSource(
        ledger_ura=_required_ura(output.get("ledger_ura"), "ledger_ura"),
    )


def _parse_record(value: object) -> InvocationLedgerRecord:
    if not isinstance(value, Mapping):
        raise _invalid("Receipt history record must be an object")
    try:
        return parse_invocation_ledger_record(value)
    except Exception as error:
        raise _invalid("invalid Axon invocation ledger projection", error) from error


def _summary_value(receipt: object, name: str) -> str:
    if isinstance(receipt, Mapping):
        value = receipt.get(name)
    else:
        value = getattr(receipt, name, None)
    if value is None:
        return ""
    return _required_text(value, name)


def _receipt_limit(limit: int) -> int:
    if not isinstance(limit, int) or isinstance(limit, bool) or limit < 0:
        raise _invalid("Receipt history limit must be non-negative")
    if limit == 0:
        return DEFAULT_RECEIPT_PAGE_LIMIT
    if limit > MAX_RECEIPT_PAGE_LIMIT:
        raise _invalid("Receipt history limit exceeds the maximum page bound")
    return limit


def _receipt_cursor(value: object) -> str:
    cursor = _optional_text(value, "cursor")
    if len(cursor) > MAX_RECEIPT_CURSOR_LENGTH:
        raise _invalid("Receipt history cursor exceeds the maximum bound")
    return cursor


def _optional_output_text(value: Mapping[str, object], name: str) -> str:
    if name not in value or value.get(name) is None:
        return ""
    projected = _required_text(value.get(name), name)
    if name == "next_cursor" and len(projected) > MAX_RECEIPT_CURSOR_LENGTH:
        raise _invalid("Receipt history next_cursor exceeds the maximum bound")
    return projected


def _mapping_text(value: Mapping[str, object], name: str) -> str:
    return _required_text(value.get(name), name)


def _required_text(value: object, name: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise _invalid(f"{name} is required")
    return value.strip()


def _optional_text(value: object, name: str) -> str:
    if not isinstance(value, str):
        raise _invalid(f"{name} must be a string")
    return value.strip()


def _required_ura(value: object, name: str) -> str:
    projected = _required_text(value, name)
    try:
        _parse_ura(projected)
    except Exception as error:
        raise _invalid(f"{name} must be a canonical URA", error) from error
    return projected


def _optional_ura(value: object, name: str) -> str:
    projected = _optional_text(value, name)
    if not projected:
        return ""
    return _required_ura(projected, name)


def _text_sequence(value: object, name: str) -> tuple[str, ...]:
    if isinstance(value, (str, bytes, bytearray)) or not isinstance(value, Sequence):
        raise _invalid(f"{name} must be a sequence of strings")
    projected = tuple(_required_text(item, f"{name} item") for item in value)
    if len(set(projected)) != len(projected):
        raise _invalid(f"{name} must not contain duplicates")
    return projected


def _ura_sequence(value: object, name: str) -> tuple[str, ...]:
    projected = _text_sequence(value, name)
    return tuple(_required_ura(item, f"{name} item") for item in projected)


def _invalid(message: str, cause: BaseException | None = None) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="receipt",
        retry=RetryHint.NEVER,
        message=message,
        cause=cause,
    )
