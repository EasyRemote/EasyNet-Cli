"""Direct daemon Runtime Core transport over Axon gRPC UDS.

This module is the Python SDK's concrete daemon Invocation transport. It
translates SDK JSON DTOs into Axon protobuf requests and delegates all runtime
semantics to the daemon endpoint.
"""

from __future__ import annotations

import base64
import binascii
import json
import secrets
from dataclasses import dataclass, field
from typing import Any, Mapping

import grpc  # type: ignore[import-untyped]

from ._axon_pb.axon.v1 import (
    invoke_pb2 as _invoke_pb2,
    invoke_pb2_grpc as _invoke_pb2_grpc,
    types_pb2 as _types_pb2,
)
from .control_ipc import ControlDiscovery, read_control_discovery
from .errors import ErrorCode, RetryHint, SDKError, normalize_error_code
from .invocation import InvocationDraft
from .runtime import RuntimeTransport

DEFAULT_URA_PROFILE = "easynet-strict-v2"
DEFAULT_DIAL_TIMEOUT_SECONDS = 3.0
DEFAULT_INVOKE_TIMEOUT_SECONDS = 60.0

invoke_pb2: Any = _invoke_pb2
invoke_pb2_grpc: Any = _invoke_pb2_grpc
types_pb2: Any = _types_pb2


@dataclass
class DirectDaemonRuntimeConnector:
    """RuntimeConnector for direct daemon Invocation gRPC over UDS."""

    control_path: str = ""
    discovery_reader: Any = read_control_discovery
    _transports: list["DirectDaemonRuntimeTransport"] = field(default_factory=list)
    _closed: bool = False

    def resolve(self, options_json: bytes) -> bytes:
        self._require_open()
        options = _decode_object(options_json, "connect options")
        endpoint = _optional_string(options.get("endpoint"), "endpoint") or ""
        control_path = (
            _optional_string(options.get("control_path"), "control_path")
            or self.control_path
        )
        facts: dict[str, object] = {
            "endpoint": endpoint,
            "control_path": control_path,
        }
        for option_name in ("dial_timeout_ms", "invoke_timeout_ms", "max_message_bytes"):
            if option_name in options:
                facts[option_name] = _optional_non_negative_int(
                    options.get(option_name),
                    option_name,
                )
        if endpoint:
            return _json_bytes(facts)

        discovery: ControlDiscovery = self.discovery_reader(control_path)
        if not discovery.invocation_endpoint:
            raise SDKError(
                code=ErrorCode.CONTROL_ONLY,
                stage="direct_runtime.resolve",
                retry=RetryHint.SAFE,
                retryable=True,
                message="control discovery did not advertise invocation_endpoint",
                details={"control_path": control_path},
            )
        facts.update(
            {
                "endpoint": discovery.invocation_endpoint,
                "control_endpoint": discovery.socket_path,
                "daemon_version": discovery.daemon_version,
                "capability_flags": list(discovery.capability_flags),
            }
        )
        return _json_bytes(facts)

    def handshake(self, endpoint_json: bytes) -> tuple[RuntimeTransport, bytes]:
        self._require_open()
        endpoint = _decode_object(endpoint_json, "runtime endpoint")
        endpoint_value = _required_string(endpoint, "endpoint")
        dial_timeout = _timeout_seconds(
            endpoint.get("dial_timeout_ms"), DEFAULT_DIAL_TIMEOUT_SECONDS
        )
        invoke_timeout = _timeout_seconds(
            endpoint.get("invoke_timeout_ms"), DEFAULT_INVOKE_TIMEOUT_SECONDS
        )
        max_message_bytes = _optional_non_negative_int(
            endpoint.get("max_message_bytes"), "max_message_bytes"
        )
        transport = DirectDaemonRuntimeTransport.open(
            endpoint_value,
            dial_timeout_seconds=dial_timeout,
            invoke_timeout_seconds=invoke_timeout,
            max_message_bytes=max_message_bytes,
        )
        self._transports.append(transport)
        facts = {
            "transport": "direct-axon-grpc-uds",
            "endpoint": endpoint_value,
            "protocol": "axon.v1.Invocation",
            "unary": True,
            "stream": False,
            "bidi": False,
            "prepare": False,
            "submit_signed": False,
        }
        return transport, _json_bytes(facts)

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        while self._transports:
            self._transports.pop().close()

    def _require_open(self) -> None:
        if self._closed:
            raise _direct_error("runtime connector is closed", code=ErrorCode.INVALID_HANDLE)


class DirectDaemonRuntimeTransport:
    """Concrete unary RuntimeTransport using daemon Axon gRPC over UDS."""

    def __init__(
        self,
        channel: grpc.Channel,
        *,
        endpoint: str,
        invoke_timeout_seconds: float,
    ) -> None:
        self._channel = channel
        self._stub = invoke_pb2_grpc.InvocationStub(channel)
        self._endpoint = endpoint
        self._invoke_timeout_seconds = invoke_timeout_seconds
        self._closed = False

    @classmethod
    def open(
        cls,
        endpoint: str,
        *,
        dial_timeout_seconds: float = DEFAULT_DIAL_TIMEOUT_SECONDS,
        invoke_timeout_seconds: float = DEFAULT_INVOKE_TIMEOUT_SECONDS,
        max_message_bytes: int = 0,
    ) -> "DirectDaemonRuntimeTransport":
        target = _grpc_uds_target(endpoint)
        options: list[tuple[str, int]] = []
        if max_message_bytes:
            options.extend(
                [
                    ("grpc.max_send_message_length", max_message_bytes),
                    ("grpc.max_receive_message_length", max_message_bytes),
                ]
            )
        channel = grpc.insecure_channel(target, options=options)
        try:
            grpc.channel_ready_future(channel).result(timeout=dial_timeout_seconds)
        except grpc.FutureTimeoutError as exc:
            _close_channel(channel)
            raise _direct_error(
                "daemon invocation endpoint is not ready",
                code=ErrorCode.DAEMON_OFFLINE,
                retry=RetryHint.SAFE,
                retryable=True,
                details={"endpoint": endpoint},
                cause=exc,
            ) from exc
        except Exception as exc:
            _close_channel(channel)
            raise _direct_error(
                f"open daemon invocation endpoint failed: {exc}",
                code=ErrorCode.TRANSPORT,
                retry=RetryHint.SAFE,
                retryable=True,
                details={"endpoint": endpoint},
                cause=exc,
            ) from exc
        return cls(
            channel,
            endpoint=endpoint,
            invoke_timeout_seconds=invoke_timeout_seconds,
        )

    def invoke(self, draft_json: bytes) -> bytes:
        self._require_open()
        try:
            draft = InvocationDraft.from_json(draft_json)
            request = _draft_to_invoke_request(draft)
            response = self._stub.Invoke(
                request,
                timeout=self._invoke_timeout_seconds,
            )
            return _invoke_response_json(draft, response)
        except SDKError:
            raise
        except grpc.RpcError as exc:
            raise _grpc_error(exc, endpoint=self._endpoint) from exc
        except Exception as exc:
            raise _direct_error(
                f"invoke daemon endpoint failed: {exc}",
                code=ErrorCode.TRANSPORT,
                retry=RetryHint.UNKNOWN,
                retryable=False,
                details={"endpoint": self._endpoint},
                cause=exc,
            ) from exc

    def open_stream(self, draft_json: bytes) -> tuple[Any, bytes]:
        raise _unsupported("direct daemon server-stream transport is not implemented")

    def open_bidi(self, draft_json: bytes, streams_json: bytes) -> tuple[Any, bytes]:
        raise _unsupported("direct daemon bidirectional transport is not implemented")

    def prepare(self, draft_json: bytes, options_json: bytes) -> bytes:
        raise _unsupported("direct daemon prepare transport is not implemented")

    def submit_signed(self, signed_json: bytes) -> bytes:
        raise _unsupported("direct daemon signed submit transport is not implemented")

    def await_handle(self, handle_id: int) -> bytes:
        raise _unsupported("direct daemon handle await transport is not implemented")

    def cancel_handle(self, handle_id: int, reason: str) -> bytes:
        raise _unsupported("direct daemon handle cancel transport is not implemented")

    def handle_events(self, handle_id: int) -> bytes:
        raise _unsupported("direct daemon handle events transport is not implemented")

    def free_handle(self, handle_id: int) -> None:
        raise _unsupported("direct daemon handle free transport is not implemented")

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        _close_channel(self._channel)

    def _require_open(self) -> None:
        if self._closed:
            raise _direct_error("runtime transport is closed", code=ErrorCode.INVALID_HANDLE)


def _draft_to_invoke_request(draft: InvocationDraft) -> Any:
    content_type = draft.content_type
    return invoke_pb2.InvokeRequest(
        envelope=types_pb2.Envelope(
            request_id=f"req-{secrets.token_hex(16)}",
            caller=_agent_identity(draft.caller_ura),
            callee=_agent_identity(draft.callee_ura),
            subject=types_pb2.SubjectIdentity(
                ura=draft.subject_ura,
                profile=DEFAULT_URA_PROFILE,
            ),
            invocation_nonce=_base64_decode(draft.nonce_base64, "nonce_base64"),
            causal_context=_causal_context(draft.causal_context),
            caller_signature=_caller_signature(draft),
        ),
        function_name=draft.descriptor_ref,
        arguments=_arguments(draft),
        content_type=content_type,
        metadata=_metadata(draft.metadata),
        content_envelope=types_pb2.ContentEnvelope(
            content_type=content_type,
            encoding="identity",
        ),
    )


def _invoke_response_json(
    draft: InvocationDraft,
    response: Any,
) -> bytes:
    terminal_state = _state_name(response.state)
    output_content_type = response.result_content_type
    output_base64 = base64.b64encode(response.result).decode("ascii")
    error = _response_failure(response, terminal_state)
    result: dict[str, object] = {
        "ok": error is None,
        "tuple": draft.to_json_dict(),
        "terminal_state": terminal_state,
        "output_content_type": output_content_type,
        "output_base64": output_base64,
        "output_json": _output_json(response.result, output_content_type),
        "selected_node_id": response.selected_node_id,
        "scheduling_reason": response.scheduling_reason,
        "elapsed_ms": response.elapsed_ms,
        "receipt": _receipt(response.terminal_receipt)
        if response.HasField("terminal_receipt")
        else None,
        "error": error,
    }
    return _json_bytes(result)


def _agent_identity(ura: str) -> Any:
    return types_pb2.AgentIdentity(ura=ura, profile=DEFAULT_URA_PROFILE)


def _caller_signature(draft: InvocationDraft) -> Any:
    signature = draft.caller_signature
    if signature is None:
        return None
    return types_pb2.CallerSignature(
        algorithm=signature.algorithm,
        signature=_base64_decode(
            signature.signature_base64,
            "caller_signature.signature_base64",
        ),
        key_id_hint=signature.key_id_hint or "",
    )


def _arguments(draft: InvocationDraft) -> bytes:
    if draft.arguments_base64 is not None:
        return _base64_decode(draft.arguments_base64, "arguments_base64")
    return json.dumps(draft.args, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _metadata(metadata: Mapping[str, object]) -> dict[str, str]:
    result: dict[str, str] = {}
    for key, value in metadata.items():
        if not isinstance(key, str) or not isinstance(value, str):
            raise _direct_error(
                "metadata must be a string-to-string map for Axon InvokeRequest",
                code=ErrorCode.INVALID_INVOCATION,
                retry=RetryHint.NEVER,
                details={"field": "metadata"},
            )
        result[key] = value
    return result


def _causal_context(value: Mapping[str, object]) -> Any:
    form = _optional_string(value.get("form"), "causal_context.form") or _optional_string(
        value.get("kind"), "causal_context.kind"
    )
    if form in (None, "", "none", "empty", "null"):
        return types_pb2.CausalContext(none=types_pb2.Empty())
    if form == "scalar":
        return types_pb2.CausalContext(scalar=_receipt_ref(value))
    if form == "list":
        prior = value.get("prior", [])
        if not isinstance(prior, list):
            raise _invalid_causal_context("causal_context.prior must be an array")
        return types_pb2.CausalContext(
            list=types_pb2.ReceiptList(prior=[_receipt_ref(item) for item in prior])
        )
    if form == "merkle":
        root_hex = _required_string(value, "root_hex")
        return types_pb2.CausalContext(
            merkle=types_pb2.MerkleRoot(
                root=_hex_decode(root_hex, "root_hex"),
                proof_ura=_required_string(value, "proof_ura"),
            )
        )
    raise _invalid_causal_context(f"unknown causal_context form: {form}")


def _receipt_ref(value: object) -> Any:
    if not isinstance(value, Mapping):
        raise _invalid_causal_context("causal receipt ref must be an object")
    receipt_hash_hex = _required_string(value, "receipt_hash_hex")
    return types_pb2.ReceiptRef(
        receipt_hash=_hex_decode(receipt_hash_hex, "receipt_hash_hex"),
        receipt_ura=_required_string(value, "receipt_ura"),
    )


def _receipt(receipt: Any) -> dict[str, object]:
    return {
        "index": receipt.index,
        "invocation_id": receipt.invocation_id,
        "receipt_type": receipt.receipt_type,
        "state": _state_name(receipt.state),
        "timestamp_unix_ms": receipt.timestamp_unix_ms,
        "prev_receipt_hash_hex": receipt.prev_receipt_hash.hex(),
        "self_hash_hex": receipt.self_hash.hex(),
        "payload_content_type": receipt.payload_content_type,
        "cleanup_complete": receipt.cleanup_complete,
        "reason": receipt.reason,
        "child_invocation_id": receipt.child_invocation_id,
    }


def _response_failure(
    response: Any,
    terminal_state: str,
) -> dict[str, object] | None:
    if response.HasField("error"):
        error = response.error
        code = _response_error_code(error.code)
        return {
            "code": code.value,
            "stage": _error_stage(error.stage),
            "message": error.message,
            "retryable": error.retryable,
        }
    if terminal_state in {"Completed", "Accepted", "Admitted", "Dispatched", "Running"}:
        return None
    code = ErrorCode.TIMEOUT if terminal_state == "TimedOut" else ErrorCode.ABILITY_FAILED
    return {
        "code": code.value,
        "stage": "direct_runtime.invoke",
        "message": f"daemon invocation ended in {terminal_state}",
        "retryable": code == ErrorCode.TIMEOUT,
    }


def _response_error_code(code: str) -> ErrorCode:
    if code:
        try:
            return normalize_error_code(code)
        except SDKError:
            return ErrorCode.ABILITY_FAILED
    return ErrorCode.ABILITY_FAILED


def _state_name(value: int) -> str:
    names = {
        types_pb2.INVOCATION_STATE_ACCEPTED: "Accepted",
        types_pb2.INVOCATION_STATE_ADMITTED: "Admitted",
        types_pb2.INVOCATION_STATE_DISPATCHED: "Dispatched",
        types_pb2.INVOCATION_STATE_RUNNING: "Running",
        types_pb2.INVOCATION_STATE_COMPLETED: "Completed",
        types_pb2.INVOCATION_STATE_FAILED: "Failed",
        types_pb2.INVOCATION_STATE_TIMED_OUT: "TimedOut",
        types_pb2.INVOCATION_STATE_CANCELLED: "Cancelled",
    }
    return names.get(value, "Unspecified")


def _error_stage(value: int) -> str:
    try:
        name = types_pb2.ErrorStage.Name(value)
    except ValueError:
        return "direct_runtime.invoke"
    return name.removeprefix("ERROR_STAGE_").lower() or "direct_runtime.invoke"


def _output_json(payload: bytes, content_type: str) -> object:
    if not payload or "json" not in content_type.lower():
        return None
    try:
        return json.loads(payload.decode("utf-8"))
    except Exception:
        return None


def _grpc_uds_target(endpoint: str) -> str:
    if endpoint.startswith("unix:"):
        return endpoint
    return f"unix:{endpoint}"


def _close_channel(channel: grpc.Channel) -> None:
    close = getattr(channel, "close", None)
    if close is not None:
        close()


def _grpc_error(error: grpc.RpcError, *, endpoint: str) -> SDKError:
    code = error.code()
    message = error.details() or str(error)
    mapping = {
        grpc.StatusCode.CANCELLED: (ErrorCode.CANCELLED, RetryHint.UNKNOWN, False),
        grpc.StatusCode.DEADLINE_EXCEEDED: (ErrorCode.TIMEOUT, RetryHint.SAFE, True),
        grpc.StatusCode.UNAVAILABLE: (ErrorCode.DAEMON_OFFLINE, RetryHint.SAFE, True),
        grpc.StatusCode.INVALID_ARGUMENT: (
            ErrorCode.INVALID_INVOCATION,
            RetryHint.NEVER,
            False,
        ),
        grpc.StatusCode.PERMISSION_DENIED: (
            ErrorCode.PERMISSION_DENIED,
            RetryHint.NEVER,
            False,
        ),
        grpc.StatusCode.NOT_FOUND: (ErrorCode.NOT_FOUND, RetryHint.NEVER, False),
        grpc.StatusCode.UNIMPLEMENTED: (
            ErrorCode.PROTOCOL_MISMATCH,
            RetryHint.NEVER,
            False,
        ),
    }
    sdk_code, retry, retryable = mapping.get(
        code,
        (ErrorCode.TRANSPORT, RetryHint.UNKNOWN, False),
    )
    return _direct_error(
        message,
        code=sdk_code,
        retry=retry,
        retryable=retryable,
        details={"endpoint": endpoint, "grpc_status": str(code)},
        cause=error,
    )


def _unsupported(message: str) -> SDKError:
    return _direct_error(
        message,
        code=ErrorCode.NOT_IMPLEMENTED,
        retry=RetryHint.NEVER,
        details={"transport": "direct-axon-grpc-uds"},
    )


def _invalid_causal_context(message: str) -> SDKError:
    return _direct_error(
        message,
        code=ErrorCode.INVALID_INVOCATION,
        retry=RetryHint.NEVER,
        details={"field": "causal_context"},
    )


def _direct_error(
    message: str,
    *,
    code: ErrorCode = ErrorCode.TRANSPORT,
    retry: RetryHint = RetryHint.NEVER,
    retryable: bool = False,
    details: Mapping[str, object] | None = None,
    cause: BaseException | None = None,
) -> SDKError:
    return SDKError(
        code=code,
        stage="direct_runtime",
        retry=retry,
        retryable=retryable,
        message=message,
        details=dict(details or {}),
        cause=cause,
    )


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _decode_object(raw: bytes, name: str) -> dict[str, object]:
    try:
        decoded = json.loads(raw.decode("utf-8"))
    except Exception as exc:
        raise _direct_error(
            f"decode {name} JSON: {exc}",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
            cause=exc,
        ) from exc
    if not isinstance(decoded, dict):
        raise _direct_error(
            f"{name} JSON must be an object",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
        )
    return decoded


def _base64_decode(value: str, field_name: str) -> bytes:
    try:
        return base64.b64decode(value.encode("ascii"), validate=True)
    except (binascii.Error, UnicodeEncodeError) as exc:
        raise _direct_error(
            f"{field_name} must be base64: {exc}",
            code=ErrorCode.INVALID_INVOCATION,
            retry=RetryHint.NEVER,
            cause=exc,
        ) from exc


def _hex_decode(value: str, field_name: str) -> bytes:
    try:
        return bytes.fromhex(value.removeprefix("sha256:"))
    except ValueError as exc:
        raise _direct_error(
            f"{field_name} must be hex: {exc}",
            code=ErrorCode.INVALID_INVOCATION,
            retry=RetryHint.NEVER,
            cause=exc,
        ) from exc


def _timeout_seconds(value: object, default: float) -> float:
    millis = _optional_non_negative_int(value, "timeout_ms")
    if millis <= 0:
        return default
    return millis / 1000.0


def _required_string(decoded: Mapping[str, object], field_name: str) -> str:
    value = decoded.get(field_name)
    if not isinstance(value, str) or value == "":
        raise _direct_error(
            f"{field_name} is required",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
        )
    return value


def _optional_string(value: object, field_name: str) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _direct_error(
            f"{field_name} must be a string",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
        )
    return value


def _optional_non_negative_int(value: object, field_name: str) -> int:
    if value is None:
        return 0
    if not isinstance(value, int) or value < 0:
        raise _direct_error(
            f"{field_name} must be a non-negative integer",
            code=ErrorCode.INVALID_ARGUMENT,
            retry=RetryHint.NEVER,
        )
    return value
