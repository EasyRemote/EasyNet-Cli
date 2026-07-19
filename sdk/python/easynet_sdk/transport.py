"""Provider-neutral Invocation transport adapters and lifecycle state machines."""

from __future__ import annotations

import base64
import json
import queue
import threading
import time
from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Callable, Iterable, Iterator, Mapping, Protocol, cast

from .bidi import BidiFrame, BidiOutcome, BidiSession, BidiState, BidiStreamDescriptor
from .connection import (
    ConnectOptions,
    RuntimeConnection,
)
from .errors import ErrorCode, RetryHint, SDKError, canonical_failure_code
from .invocation import InvocationDraft
from .runtime import (
    InvocationFailure,
    InvocationHandle,
    InvocationResult,
    PrepareOptions,
    RuntimeClient,
    RuntimeReceipt,
)
from .signing import Signer
from .stream import StreamCancel, StreamEvent, StreamHandle


class _RuntimeInvocationTransportState(Enum):
    OPEN = "open"
    CLOSING = "closing"
    CLOSE_RETRYABLE = "close_retryable"
    CLOSE_FAILED = "close_failed"
    CLOSED = "closed"


class _RuntimeUseLease:
    def __init__(
        self,
        runtime: RuntimeClient,
        release: Callable[[], None],
    ) -> None:
        self._runtime = runtime
        self._release = release
        self._lock = threading.Lock()
        self._released = False

    def __enter__(self) -> RuntimeClient:
        return self._runtime

    def __exit__(self, *exc_info: object) -> None:
        self.release()

    def release(self) -> None:
        with self._lock:
            if self._released:
                return
            self._released = True
        self._release()


@dataclass
class RuntimeInvocationTransport:
    """JSON-friendly facade over one canonical RuntimeClient."""

    runtime: RuntimeClient
    connection: RuntimeConnection | None = None
    _closed: bool = False
    _state: _RuntimeInvocationTransportState = field(
        default=_RuntimeInvocationTransportState.OPEN,
        init=False,
        repr=False,
    )
    _lifecycle: threading.Condition = field(
        default_factory=threading.Condition,
        init=False,
        repr=False,
    )
    _active_uses: int = field(default=0, init=False, repr=False)
    _close_error: BaseException | None = field(default=None, init=False, repr=False)
    _retained_handles: dict[int, InvocationHandle] = field(
        default_factory=dict,
        init=False,
        repr=False,
    )

    @classmethod
    def from_runtime_client(
        cls, runtime: RuntimeClient
    ) -> "RuntimeInvocationTransport":
        """Wrap an existing Runtime Core client."""

        return cls(runtime)

    @classmethod
    def connect(
        cls,
        *,
        control_path: str = "",
        library_path: str | None = None,
        options: ConnectOptions = ConnectOptions(),
    ) -> "RuntimeInvocationTransport":
        """REQ-LANG-5 delegate to the explicit EasyNet C ABI provider."""

        from .providers.easynet.transport import connect_invocation_transport

        return connect_invocation_transport(
            control_path=control_path,
            library_path=library_path,
            options=options,
        )

    @classmethod
    def connect_direct(
        cls,
        *,
        control_path: str = "",
        library_path: str | None = None,
        options: ConnectOptions = ConnectOptions(),
        identity: Any | None = None,
    ) -> "RuntimeInvocationTransport":
        """REQ-LANG-5 delegate to the explicit EasyNet direct provider."""

        from .providers.easynet.transport import connect_direct_invocation_transport

        return connect_direct_invocation_transport(
            control_path=control_path,
            library_path=library_path,
            options=options,
            identity=identity,
        )

    def invoke(
        self, invocation: Mapping[str, object] | InvocationDraft
    ) -> dict[str, object]:
        """Submit one complete Invocation and return its Runtime result JSON."""

        with self._acquire_runtime_use() as runtime:
            result = runtime.invoke(_coerce_draft(invocation))
        return _invocation_result_dict(result)

    def invoke_signed(
        self,
        invocation: Mapping[str, object] | InvocationDraft,
        *,
        signer: Signer | None,
        options: PrepareOptions = PrepareOptions(local_daemon_signing=True),
    ) -> dict[str, object]:
        """Prepare, sign, submit, await, and release one signed Invocation."""

        if signer is None:
            raise _missing_required_signer()
        with self._acquire_runtime_use() as runtime:
            signed, _material = runtime.prepare_and_sign(
                _coerce_draft(invocation),
                signer,
                options,
            )
            handle = runtime.submit_signed(signed)
            try:
                result = runtime.await_result(handle)
            except BaseException as operation_error:
                try:
                    self._close_invocation_handle(runtime, handle)
                except BaseException as cleanup_error:
                    raise operation_error from cleanup_error
                raise
            self._close_invocation_handle(runtime, handle)
            return _invocation_result_dict(result)

    def stream(
        self, invocation: Mapping[str, object] | InvocationDraft
    ) -> "RuntimeFrameStream":
        """Open a server-stream Invocation."""

        with self._acquire_runtime_use() as runtime:
            handle = runtime.invoke_stream(_coerce_draft(invocation))
        return RuntimeFrameStream(handle)

    def bidi(
        self,
        invocation: Mapping[str, object] | InvocationDraft,
        streams: Iterable[Mapping[str, object] | BidiStreamDescriptor] = (),
    ) -> "RuntimeBidiChannel":
        """Open a bidirectional Invocation session."""

        with self._acquire_runtime_use() as runtime:
            session = runtime.open_bidi(
                _coerce_draft(invocation),
                tuple(_coerce_stream_descriptor(stream) for stream in streams),
            )
        return RuntimeBidiChannel(session)

    def close(self) -> None:
        while True:
            with self._lifecycle:
                if self._state is _RuntimeInvocationTransportState.CLOSED:
                    return
                if self._state is _RuntimeInvocationTransportState.CLOSE_FAILED:
                    assert self._close_error is not None
                    raise self._close_error
                if self._state is _RuntimeInvocationTransportState.CLOSING:
                    self._lifecycle.wait()
                    continue
                self._state = _RuntimeInvocationTransportState.CLOSING
                while self._active_uses:
                    self._lifecycle.wait()
                break

        try:
            self._close_retained_handles()
        except BaseException as exc:
            self._finish_close_failure(exc, retryable=True)
            raise

        try:
            if self.connection is not None:
                self.connection.close()
            else:
                self.runtime.close()
        except BaseException as exc:
            # RuntimeClient and RuntimeConnection poison their own close state
            # before raising. Retain and replay the failure instead of treating
            # a later delegated no-op as successful ownership release.
            self._finish_close_failure(exc, retryable=False)
            raise

        with self._lifecycle:
            self._state = _RuntimeInvocationTransportState.CLOSED
            self._closed = True
            self._close_error = None
            self._lifecycle.notify_all()

    def __enter__(self) -> "RuntimeInvocationTransport":
        with self._lifecycle:
            self._require_open_locked()
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()

    def _acquire_runtime_use(self) -> _RuntimeUseLease:
        with self._lifecycle:
            self._require_open_locked()
            self._active_uses += 1
        return _RuntimeUseLease(self.runtime, self._release_runtime_use)

    def _release_runtime_use(self) -> None:
        with self._lifecycle:
            self._active_uses -= 1
            self._lifecycle.notify_all()

    def _close_invocation_handle(
        self,
        runtime: RuntimeClient,
        handle: InvocationHandle,
    ) -> None:
        try:
            runtime.close_handle(handle)
        except BaseException:
            with self._lifecycle:
                self._retained_handles[
                    handle.control_capability()._adapter_handle_id()
                ] = handle
            raise
        with self._lifecycle:
            self._retained_handles.pop(
                handle.control_capability()._adapter_handle_id(), None
            )

    def _close_retained_handles(self) -> None:
        with self._lifecycle:
            handles = tuple(self._retained_handles.values())
        failures: list[BaseException] = []
        for handle in handles:
            try:
                self.runtime.close_handle(handle)
            except BaseException as exc:
                failures.append(exc)
            else:
                with self._lifecycle:
                    self._retained_handles.pop(
                        handle.control_capability()._adapter_handle_id(), None
                    )
        if len(failures) == 1:
            raise failures[0]
        if failures:
            raise BaseExceptionGroup("invocation handle cleanup failed", failures)

    def _finish_close_failure(
        self,
        error: BaseException,
        *,
        retryable: bool,
    ) -> None:
        with self._lifecycle:
            self._close_error = error
            self._state = (
                _RuntimeInvocationTransportState.CLOSE_RETRYABLE
                if retryable
                else _RuntimeInvocationTransportState.CLOSE_FAILED
            )
            self._lifecycle.notify_all()

    def _require_open_locked(self) -> None:
        if self._state is not _RuntimeInvocationTransportState.OPEN:
            raise _closed_transport("daemon invocation transport is closing or closed")


def _open_runtime_invocation_transport(
    connection: RuntimeConnection,
    options: ConnectOptions,
) -> RuntimeInvocationTransport:
    try:
        connection.connect(options)
        runtime = connection.runtime_client()
    except BaseException as acquisition_error:
        try:
            connection.close()
        except BaseException as cleanup_error:
            raise acquisition_error from cleanup_error
        raise
    return RuntimeInvocationTransport(runtime=runtime, connection=connection)


@dataclass
class InvocationResultAdapter:
    """Runtime result adapter over the canonical Invocation transport."""

    transport: RuntimeInvocationTransport

    @classmethod
    def from_runtime_client(cls, runtime: RuntimeClient) -> "InvocationResultAdapter":
        return cls(RuntimeInvocationTransport.from_runtime_client(runtime))

    @classmethod
    def connect(
        cls,
        *,
        control_path: str = "",
        library_path: str | None = None,
        options: ConnectOptions = ConnectOptions(),
    ) -> "InvocationResultAdapter":
        return cls(
            RuntimeInvocationTransport.connect(
                control_path=control_path,
                library_path=library_path,
                options=options,
            )
        )

    def invoke(
        self, invocation: Mapping[str, object] | InvocationDraft
    ) -> dict[str, object]:
        """Submit one complete Invocation and return runtime result adapter shape."""

        return _result_response_dict(self.transport.invoke(invocation))

    def invoke_signed(
        self,
        invocation: Mapping[str, object] | InvocationDraft,
        *,
        signer: Signer | None,
        options: PrepareOptions = PrepareOptions(local_daemon_signing=True),
    ) -> dict[str, object]:
        """Submit a signed Invocation and return runtime result adapter shape."""

        return _result_response_dict(
            self.transport.invoke_signed(invocation, signer=signer, options=options)
        )

    def stream(
        self, invocation: Mapping[str, object] | InvocationDraft
    ) -> "RuntimeFrameStream":
        return self.transport.stream(invocation)

    def bidi(
        self,
        invocation: Mapping[str, object] | InvocationDraft,
        streams: Iterable[Mapping[str, object] | BidiStreamDescriptor] = (),
    ) -> "RuntimeBidiChannel":
        return self.transport.bidi(invocation, streams)

    def close(self) -> None:
        self.transport.close()

    def __enter__(self) -> "InvocationResultAdapter":
        self.transport.__enter__()
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()


class UnaryInvocationTransport(Protocol):
    """Minimal unary transport contract owned by the SDK dispatch pool."""

    def invoke(
        self, invocation: Mapping[str, object] | InvocationDraft
    ) -> Mapping[str, object]:
        """Submit one runtime-shaped unary Invocation."""

    def invoke_signed(
        self,
        invocation: Mapping[str, object] | InvocationDraft,
        *,
        signer: Signer | None,
        options: PrepareOptions = PrepareOptions(local_daemon_signing=True),
    ) -> Mapping[str, object]:
        """Submit one runtime-shaped signed unary Invocation."""

    def close(self) -> None:
        """Release the underlying daemon transport."""


class _UnaryDispatchState(Enum):
    QUEUED = "queued"
    DISPATCHING = "dispatching"
    COMPLETED = "completed"
    TIMED_OUT_BEFORE_DISPATCH = "timed_out_before_dispatch"
    TIMED_OUT_AFTER_DISPATCH = "timed_out_after_dispatch"


class _UnaryPoolState(Enum):
    OPEN = "open"
    CLOSING = "closing"
    QUIESCENT = "quiescent"
    CLOSED = "closed"


class _UnaryTimeoutBudget:
    """One monotonic deadline shared by lock acquisition and result wait."""

    def __init__(self, timeout: float | None) -> None:
        if timeout is not None and timeout < 0:
            raise ValueError("'timeout' must be a non-negative number")
        self._deadline = None if timeout is None else time.monotonic() + timeout

    def acquire(self, lock: Any) -> bool:
        remaining = self.remaining()
        if remaining is None:
            lock.acquire()
            return True
        return cast(bool, lock.acquire(timeout=remaining))

    def remaining(self) -> float | None:
        if self._deadline is None:
            return None
        return max(0.0, self._deadline - time.monotonic())


@dataclass(frozen=True)
class _UnaryTimeoutOutcome:
    state: _UnaryDispatchState
    transport: UnaryInvocationTransport | None

    @property
    def execution_started(self) -> bool:
        return self.state is _UnaryDispatchState.TIMED_OUT_AFTER_DISPATCH


class _UnaryDispatchAttempt:
    """Thread-safe lifecycle for one caller wait and its worker dispatch."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._state = _UnaryDispatchState.QUEUED
        self._transport: UnaryInvocationTransport | None = None

    def begin(self, transport: UnaryInvocationTransport) -> bool:
        with self._lock:
            if self._state is _UnaryDispatchState.TIMED_OUT_BEFORE_DISPATCH:
                return False
            if self._state is not _UnaryDispatchState.QUEUED:
                raise RuntimeError(
                    f"cannot dispatch unary call from {self._state.value}"
                )
            self._transport = transport
            self._state = _UnaryDispatchState.DISPATCHING
            return True

    def is_queued(self) -> bool:
        with self._lock:
            return self._state is _UnaryDispatchState.QUEUED

    def complete(self) -> None:
        with self._lock:
            if self._state is _UnaryDispatchState.DISPATCHING:
                self._state = _UnaryDispatchState.COMPLETED
                return
            if self._state is _UnaryDispatchState.TIMED_OUT_AFTER_DISPATCH:
                return
            raise RuntimeError(f"cannot complete unary call from {self._state.value}")

    def timeout(
        self,
    ) -> _UnaryTimeoutOutcome:
        with self._lock:
            if self._state is _UnaryDispatchState.QUEUED:
                self._state = _UnaryDispatchState.TIMED_OUT_BEFORE_DISPATCH
                return _UnaryTimeoutOutcome(self._state, None)

            if self._state not in {
                _UnaryDispatchState.DISPATCHING,
                _UnaryDispatchState.COMPLETED,
            }:
                raise RuntimeError(
                    f"cannot time out unary call from {self._state.value}"
                )

            transport = self._transport
            if transport is None:
                raise RuntimeError("dispatched unary call has no transport")
            self._state = _UnaryDispatchState.TIMED_OUT_AFTER_DISPATCH
            return _UnaryTimeoutOutcome(self._state, transport)


@dataclass
class _UnaryCloseAttempt:
    completed: bool = False
    error: BaseException | None = None


class UnaryDispatchPool:
    """SDK-owned single-flight unary wait/retire state machine.

    Callers acquire the flight lease before creating a worker. Owned transports
    are published through the pool lifecycle, retired after timed-out dispatch,
    and retained for retry if closing them fails.
    """

    def __init__(
        self,
        transport_factory: Callable[[], UnaryInvocationTransport],
        *,
        owned: bool = True,
    ) -> None:
        self._transport_factory = transport_factory
        self._owned = owned
        self._lock = threading.Lock()
        self._lifecycle = threading.Condition(self._lock)
        self._flight_lock = threading.Lock()
        self._state = _UnaryPoolState.OPEN
        self._generation = 0
        self._active_invocations = 0
        self._transport: UnaryInvocationTransport | None = None
        self._pending_factories = 0
        self._retired: dict[int, UnaryInvocationTransport] = {}
        self._closing_transports: set[int] = set()
        self._close_failures: list[BaseException] = []
        self._cleanup_worker_running = False
        self._delegated_close: _UnaryCloseAttempt | None = None
        self._terminal_close_requested = False

    @classmethod
    def from_transport(cls, transport: UnaryInvocationTransport) -> "UnaryDispatchPool":
        """Wrap an externally-owned transport without closing or retiring it."""

        return cls(lambda: transport, owned=False)

    def invoke(
        self,
        invocation: Mapping[str, object] | InvocationDraft,
        *,
        timeout: float | None = None,
    ) -> dict[str, object]:
        return self._invoke_with_transport(
            lambda transport: transport.invoke(invocation),
            timeout=timeout,
        )

    def invoke_signed(
        self,
        invocation: Mapping[str, object] | InvocationDraft,
        *,
        signer: Signer | None,
        options: PrepareOptions = PrepareOptions(local_daemon_signing=True),
        timeout: float | None = None,
    ) -> dict[str, object]:
        return self._invoke_with_transport(
            lambda transport: transport.invoke_signed(
                invocation,
                signer=signer,
                options=options,
            ),
            timeout=timeout,
        )

    def _invoke_with_transport(
        self,
        operation: Callable[[UnaryInvocationTransport], Mapping[str, object]],
        *,
        timeout: float | None,
    ) -> dict[str, object]:
        budget = _UnaryTimeoutBudget(timeout)
        generation = self._admit_invocation()
        if not budget.acquire(self._flight_lock):
            self._validate_admission(generation)
            raise _unary_wait_timeout(timeout, execution_started=False) from None

        invocation_started = False
        try:
            self._start_invocation(generation)
            invocation_started = True
            result: queue.Queue[tuple[bool, Mapping[str, object] | BaseException]] = (
                queue.Queue(maxsize=1)
            )
            attempt = _UnaryDispatchAttempt()

            def invoke_on_transport() -> None:
                transport: UnaryInvocationTransport | None = None
                outcome: tuple[bool, Mapping[str, object] | BaseException] | None = None
                try:
                    if not attempt.is_queued():
                        return
                    transport = self._connected()
                    if not self._begin_dispatch(attempt, transport):
                        return
                    try:
                        outcome = (
                            True,
                            operation(transport),
                        )
                    except BaseException as exc:
                        outcome = (False, exc)
                    finally:
                        attempt.complete()
                except BaseException as exc:
                    outcome = (False, exc)
                finally:
                    try:
                        if outcome is not None:
                            result.put(outcome)
                    finally:
                        self._finish_invocation()
                        self._flight_lock.release()
                    if transport is not None:
                        self._schedule_cleanup()

            worker = threading.Thread(
                target=invoke_on_transport,
                name="easynet-sdk-unary",
                daemon=True,
            )
            worker.start()
        except BaseException:
            if invocation_started:
                self._finish_invocation()
            self._flight_lock.release()
            raise

        try:
            ok, payload = result.get(timeout=budget.remaining())
        except queue.Empty:
            timeout_outcome = self._timeout_attempt(attempt)
            raise _unary_wait_timeout(
                timeout, timeout_outcome.execution_started
            ) from None
        if not ok:
            assert isinstance(payload, BaseException)
            raise payload
        return dict(cast(Mapping[str, object], payload))

    def close(self) -> None:
        """Permanently close the pool without blocking on active work."""

        with self._lifecycle:
            if self._state is _UnaryPoolState.CLOSED:
                return
            self._terminal_close_requested = True
            if self._state is _UnaryPoolState.QUIESCENT:
                self._state = _UnaryPoolState.CLOSED
                return
        self.quiesce()

    def quiesce(self) -> None:
        """Release the current generation while keeping the pool reusable.

        Quiesce never waits for an active worker or factory. The bounded cleanup
        coordinator resolves retired transports; an idle close resolves them
        synchronously and reports all failures. The next use opens a generation.
        """

        with self._lifecycle:
            if self._state is _UnaryPoolState.CLOSED:
                return
            if self._state is _UnaryPoolState.OPEN:
                self._begin_closing_locked()
            if self._state is _UnaryPoolState.QUIESCENT:
                return
            if not self._owned or self._active_invocations != 0:
                return

            close_attempt = self._delegated_close
            if close_attempt is not None and not close_attempt.completed:
                while not close_attempt.completed:
                    self._lifecycle.wait()
                delegated_error = close_attempt.error
                if delegated_error is not None:
                    raise delegated_error
                return

            close_attempt = _UnaryCloseAttempt()
            self._delegated_close = close_attempt

        error: BaseException | None = None
        try:
            self._close_all_retired()
            error = self._take_recorded_close_error()
        except BaseException as exc:
            error = exc
        finally:
            with self._lifecycle:
                close_attempt.error = error
                close_attempt.completed = True
                self._lifecycle.notify_all()

        if error is not None:
            raise error

    @property
    def current_transport(self) -> UnaryInvocationTransport | None:
        """Return the current reusable transport for tests/diagnostics."""

        with self._lock:
            return self._transport

    def connected_transport(self) -> UnaryInvocationTransport:
        """Return the current reusable transport, opening one if needed."""

        with self._lock:
            if self._state is _UnaryPoolState.QUIESCENT:
                self._state = _UnaryPoolState.OPEN
            self._require_open_locked()
        return self._connected()

    def _connected(self) -> UnaryInvocationTransport:
        with self._lock:
            self._require_open_locked()
            if self._owned and self._transport is not None:
                return self._transport
            self._pending_factories += 1

        try:
            candidate = self._transport_factory()
        except BaseException:
            with self._lock:
                self._pending_factories -= 1
                self._finish_closing_locked()
            raise

        winner: UnaryInvocationTransport | None = None
        close_candidate = False
        with self._lock:
            self._pending_factories -= 1
            if self._state is _UnaryPoolState.OPEN:
                if not self._owned:
                    winner = candidate
                elif self._transport is None:
                    self._transport = candidate
                    winner = candidate
                elif self._transport is candidate:
                    winner = candidate
                else:
                    winner = self._transport
                    self._retire_locked(candidate)
                    close_candidate = True
            elif self._owned:
                self._retire_locked(candidate)
                close_candidate = True
            self._finish_closing_locked()

        if close_candidate:
            self._schedule_cleanup()
        with self._lock:
            if winner is not None and self._state is _UnaryPoolState.OPEN:
                return winner
        if winner is None:
            raise _closed_transport("unary dispatch pool is closing")
        raise _closed_transport("unary dispatch pool cleanup is pending")

    def _begin_dispatch(
        self,
        attempt: _UnaryDispatchAttempt,
        transport: UnaryInvocationTransport,
    ) -> bool:
        with self._lock:
            self._require_open_locked()
            return attempt.begin(transport)

    def _timeout_attempt(
        self,
        attempt: _UnaryDispatchAttempt,
    ) -> _UnaryTimeoutOutcome:
        with self._lock:
            outcome = attempt.timeout()
            if outcome.execution_started:
                assert outcome.transport is not None
                self._retire_locked(outcome.transport)
            return outcome

    def _retire_locked(self, transport: UnaryInvocationTransport) -> None:
        if not self._owned:
            return
        if self._transport is transport:
            self._transport = None
        self._retired.setdefault(id(transport), transport)

    def _close_retired(
        self,
        transport: UnaryInvocationTransport,
        *,
        wait: bool = False,
    ) -> bool:
        if not self._owned:
            return True

        transport_id = id(transport)
        with self._lifecycle:
            if transport_id not in self._retired:
                return True
            if transport_id in self._closing_transports:
                if not wait:
                    return True
                while transport_id in self._closing_transports:
                    self._lifecycle.wait()
                return transport_id not in self._retired
            self._closing_transports.add(transport_id)

        try:
            transport.close()
        except BaseException as exc:
            with self._lifecycle:
                self._closing_transports.remove(transport_id)
                self._close_failures.append(exc)
                self._begin_closing_locked()
                self._lifecycle.notify_all()
            return False

        with self._lifecycle:
            self._closing_transports.remove(transport_id)
            self._retired.pop(transport_id, None)
            self._finish_closing_locked()
            self._lifecycle.notify_all()
        return True

    def _schedule_cleanup(self) -> None:
        if not self._owned:
            return
        with self._lock:
            if (
                self._cleanup_worker_running
                or self._close_failures
                or not self._retired
            ):
                return
            self._cleanup_worker_running = True

        worker = threading.Thread(
            target=self._cleanup_retired,
            name="easynet-sdk-unary-cleanup",
            daemon=True,
        )
        try:
            worker.start()
        except BaseException as exc:
            with self._lock:
                self._cleanup_worker_running = False
                self._close_failures.append(exc)
                self._begin_closing_locked()

    def _cleanup_retired(self) -> None:
        try:
            while True:
                with self._lock:
                    if self._close_failures:
                        return
                    transport = next(
                        (
                            candidate
                            for transport_id, candidate in self._retired.items()
                            if transport_id not in self._closing_transports
                        ),
                        None,
                    )
                if transport is None or not self._close_retired(transport):
                    return
        finally:
            restart = False
            with self._lock:
                self._cleanup_worker_running = False
                self._finish_closing_locked()
                restart = not self._close_failures and any(
                    transport_id not in self._closing_transports
                    for transport_id in self._retired
                )
            if restart:
                self._schedule_cleanup()

    def _close_all_retired(self) -> None:
        with self._lock:
            transports = tuple(self._retired.values())
        for transport in transports:
            self._close_retired(transport, wait=True)

    def _take_recorded_close_error(self) -> BaseException | None:
        with self._lock:
            failures = tuple(self._close_failures)
            self._close_failures.clear()
            self._finish_closing_locked()
        if len(failures) == 1:
            return failures[0]
        if failures:
            return BaseExceptionGroup("unary transport cleanup failed", failures)
        return None

    def _begin_closing_locked(self) -> None:
        if self._state is not _UnaryPoolState.OPEN:
            return
        self._generation += 1
        self._state = _UnaryPoolState.CLOSING
        if self._transport is not None:
            self._retire_locked(self._transport)
        self._finish_closing_locked()

    def _finish_closing_locked(self) -> None:
        if (
            self._state is _UnaryPoolState.CLOSING
            and self._active_invocations == 0
            and self._pending_factories == 0
            and not self._retired
            and not self._closing_transports
            and not self._close_failures
        ):
            self._state = (
                _UnaryPoolState.CLOSED
                if self._terminal_close_requested
                else _UnaryPoolState.QUIESCENT
            )

    def _admit_invocation(self) -> int:
        with self._lock:
            if self._state is _UnaryPoolState.QUIESCENT:
                self._state = _UnaryPoolState.OPEN
            self._require_open_locked()
            return self._generation

    def _validate_admission(self, generation: int) -> None:
        with self._lock:
            if (
                self._state is not _UnaryPoolState.OPEN
                or generation != self._generation
            ):
                raise _closed_transport("unary dispatch generation was closed")

    def _start_invocation(self, generation: int) -> None:
        with self._lock:
            if generation != self._generation:
                raise _closed_transport("unary dispatch generation was closed")
            self._require_open_locked()
            self._active_invocations += 1

    def _finish_invocation(self) -> None:
        with self._lock:
            self._active_invocations -= 1
            self._finish_closing_locked()

    def _require_open_locked(self) -> None:
        if self._state is not _UnaryPoolState.OPEN:
            raise _closed_transport("unary dispatch pool is closing")


def _unary_wait_timeout(timeout: float | None, execution_started: bool) -> SDKError:
    if execution_started:
        return SDKError(
            code=ErrorCode.TIMEOUT,
            stage="runtime_transport",
            retry=RetryHint.UNKNOWN,
            retryable=False,
            message=(
                f"no response within {timeout}s; the invocation was dispatched, "
                "so its execution outcome is unknown; server-side timeout_seconds "
                "still governs completion"
            ),
            details={
                "reason": "client_wait_timeout",
                "timeout_seconds": timeout,
                "execution_state": "unknown",
            },
        )
    return SDKError(
        code=ErrorCode.TIMEOUT,
        stage="runtime_transport",
        retry=RetryHint.SAFE,
        retryable=True,
        message=f"no response within {timeout}s; the invocation was not dispatched",
        details={
            "reason": "client_wait_timeout",
            "timeout_seconds": timeout,
            "execution_state": "not_started",
        },
    )


@dataclass(frozen=True)
class StreamValue:
    """One SDK-projected stream item."""

    value: Any


class StreamValueAdapter:
    """SDK-owned stream frame projection.

    The adapter consumes generic daemon stream frames and yields
    ability values. It keeps terminal-frame, timeout, wire-error, and payload
    projection rules out of product facades.
    """

    _NO_VALUE = object()

    def __init__(self, frames: "FrameStream", *, timeout: float | None = None) -> None:
        self._frames = frames
        self._timeout = timeout

    def __iter__(self) -> Iterator[StreamValue]:
        try:
            for frame in self._raw_frames():
                error = frame.get("error")
                if error:
                    raise _remote_wire_error(error)
                value = self._frame_value(frame)
                stream_error = _stream_error_payload(value)
                if stream_error is not None:
                    raise _remote_wire_error(stream_error)
                if value is not self._NO_VALUE:
                    yield StreamValue(value)
                if frame.get("terminal") is True:
                    return
        finally:
            self.close()

    def close(self) -> None:
        self._frames.close()

    def _raw_frames(self) -> Iterator[Mapping[str, object]]:
        recv = getattr(self._frames, "recv", None)
        if not callable(recv):
            yield from self._frames
            return
        while True:
            try:
                frame = recv(timeout=self._timeout)
            except TimeoutError:
                raise SDKError(
                    code=ErrorCode.TIMEOUT,
                    stage="stream",
                    retry=RetryHint.SAFE,
                    retryable=True,
                    message=(
                        f"no stream frame within {self._timeout}s — the server-side "
                        "execution is still governed by the ability's timeout_seconds"
                    ),
                    details={
                        "reason": "client_wait_timeout",
                        "timeout_seconds": self._timeout,
                    },
                ) from None
            if frame is None:
                return
            yield frame

    def _frame_value(self, frame: Mapping[str, object]) -> Any:
        if (
            frame.get("terminal") is True
            and frame.get("payload_json") is None
            and not frame.get("payload_base64")
        ):
            return self._NO_VALUE
        encoded = frame.get("payload_base64")
        if "payload_json" in frame and (
            frame.get("payload_json") is not None
            or (
                frame.get("content_type") == "application/json"
                and isinstance(encoded, str)
                and bool(encoded)
            )
        ):
            return frame["payload_json"]
        if isinstance(encoded, str) and encoded:
            try:
                return base64.b64decode(encoded)
            except Exception as exc:
                raise SDKError(
                    code=ErrorCode.INVALID_ARGUMENT,
                    stage="stream",
                    retry=RetryHint.NEVER,
                    retryable=False,
                    message=f"decode stream payload_base64: {exc}",
                    cause=exc,
                ) from exc
        return self._NO_VALUE


class FrameStream(Protocol):
    """Frame stream shape consumed by `StreamValueAdapter`."""

    def recv(self, timeout: float | None = None) -> Mapping[str, object] | None: ...

    def close(self) -> None: ...

    def __iter__(self) -> Iterator[Mapping[str, object]]: ...


class BidiChannel(Protocol):
    """Bidi channel shape consumed by `BidiSessionAdapter`."""

    def send(self, frame: Mapping[str, object]) -> object: ...

    def recv(self, timeout: float | None = None) -> Mapping[str, object] | None: ...

    def close(self) -> None: ...

    def cancel(self, reason: str = "") -> object: ...


class BidiSessionAdapter:
    """SDK-owned bidi session facade.

    Public session API is intentionally small, but the lifecycle
    rules are Runtime Core concerns: close releases local carrier resources
    without claiming canonical cancellation, timeout is a typed client wait
    expiry, and remote wire errors must not leak as ordinary frames.
    """

    def __init__(
        self,
        channel: BidiChannel,
        *,
        close_reason: str = "client close",
    ) -> None:
        self._channel = channel
        self._close_reason = close_reason
        self._terminal = False
        self._closed = False

    def send(self, frame: Mapping[str, object]) -> None:
        self._require_not_closed()
        self._channel.send(frame)

    def recv(self, timeout: float | None = None) -> dict[str, object] | None:
        self._require_not_closed()
        try:
            frame = self._channel.recv(timeout=timeout)
        except StopIteration:
            self._terminal = True
            return None
        except TimeoutError:
            raise SDKError(
                code=ErrorCode.TIMEOUT,
                stage="bidi",
                retry=RetryHint.SAFE,
                retryable=True,
                message=(
                    f"no bidi frame within {timeout}s - the server-side session "
                    "is still governed by daemon/ability policy"
                ),
                details={
                    "reason": "client_wait_timeout",
                    "timeout_seconds": timeout,
                },
            ) from None
        if frame is None:
            self._terminal = True
            return None
        projected = dict(frame)
        error = projected.get("error")
        if error:
            raise _remote_wire_error(error, stage="bidi")
        if projected.get("terminal") is True:
            self._terminal = True
        return projected

    def cancel(self, reason: str = "client cancel") -> None:
        if self._closed or self._terminal:
            return
        self._channel.cancel(reason)

    def close(self) -> None:
        if self._closed:
            return
        self._channel.close()
        self._closed = True

    def __enter__(self) -> "BidiSessionAdapter":
        self._require_not_closed()
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()

    def _require_not_closed(self) -> None:
        if self._closed:
            raise SDKError(
                code=ErrorCode.CANCELLED,
                stage="bidi",
                retry=RetryHint.NEVER,
                retryable=False,
                message="bidi session is closed",
            )


@dataclass
class RuntimeFrameStream:
    """JSON-friendly canonical server-stream wrapper over ``StreamHandle``."""

    handle: StreamHandle

    def recv(self, timeout: float | None = None) -> dict[str, object]:
        return _stream_event_dict(self.handle.next(timeout))

    def cancel(self, reason: str = "") -> dict[str, object]:
        return _stream_cancel_dict(self.handle.cancel(reason))

    def close(self) -> None:
        self.handle.close()

    def __iter__(self) -> Iterator[dict[str, object]]:
        while True:
            event = self.recv()
            yield event
            if event.get("terminal") is True:
                return

    def __enter__(self) -> "RuntimeFrameStream":
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()


@dataclass
class RuntimeBidiChannel:
    """JSON-friendly canonical bidi wrapper over ``BidiSession``."""

    session: BidiSession

    def send(self, frame: Mapping[str, object] | BidiFrame) -> dict[str, object]:
        return _bidi_frame_dict(self.session.send(_coerce_bidi_frame(frame)))

    def recv(self, timeout: float | None = None) -> dict[str, object]:
        return _bidi_frame_dict(self.session.receive(timeout))

    def receive(self, timeout: float | None = None) -> dict[str, object]:
        return self.recv(timeout)

    def close_send(self) -> dict[str, object]:
        return _bidi_outcome_dict(self.session.close_send())

    def cancel(self, reason: str = "") -> dict[str, object]:
        return _bidi_outcome_dict(self.session.cancel(reason))

    def close(self) -> None:
        self.session.close()

    def __enter__(self) -> "RuntimeBidiChannel":
        return self

    def __exit__(self, *exc_info: object) -> None:
        if self.session.state not in {
            BidiState.CANCEL_REQUESTED,
            BidiState.TERMINAL,
            BidiState.CANCELLED,
            BidiState.FAILED,
            BidiState.CLOSED,
        }:
            self.session.cancel("context manager exit")
        self.close()


def _coerce_draft(
    invocation: Mapping[str, object] | InvocationDraft,
) -> InvocationDraft:
    if isinstance(invocation, InvocationDraft):
        return invocation
    if not isinstance(invocation, Mapping):
        raise _invalid_transport("invocation must be a mapping or InvocationDraft")
    return InvocationDraft.from_json(_json_bytes(dict(invocation)))


def _coerce_stream_descriptor(
    stream: Mapping[str, object] | BidiStreamDescriptor,
) -> BidiStreamDescriptor:
    if isinstance(stream, BidiStreamDescriptor):
        return stream
    if not isinstance(stream, Mapping):
        raise _invalid_transport("bidi stream descriptor must be a mapping")
    stream_id = stream.get("stream_id")
    if not isinstance(stream_id, int) or isinstance(stream_id, bool) or stream_id <= 0:
        raise _invalid_transport("bidi stream_id is required")
    return BidiStreamDescriptor(
        stream_id=stream_id,
        content_type=_optional_string(stream.get("content_type"), "content_type"),
        codec_params=_optional_string(stream.get("codec_params"), "codec_params"),
        ordering=_optional_string(stream.get("ordering"), "ordering"),
    )


def _coerce_bidi_frame(frame: Mapping[str, object] | BidiFrame) -> BidiFrame:
    if isinstance(frame, BidiFrame):
        return frame
    if not isinstance(frame, Mapping):
        raise _invalid_transport("bidi frame must be a mapping or BidiFrame")
    return BidiFrame.from_json(_json_bytes(dict(frame)))


def _invocation_result_dict(result: InvocationResult) -> dict[str, object]:
    value: dict[str, object] = {
        "ok": result.ok,
        "tuple": result.tuple.to_json_dict(),
        "terminal_state": result.terminal_state,
        "output_content_type": result.output_content_type,
        "output_base64": result.output_base64,
        "output_json": result.output_json,
        "elapsed_ms": result.elapsed_ms,
        "admission_receipt": (
            dict(result.admission_receipt)
            if result.admission_receipt is not None
            else None
        ),
        "admission_receipt_summary": (
            _runtime_receipt_dict(result.admission_receipt_summary)
            if result.admission_receipt_summary is not None
            else None
        ),
        "terminal_receipt": (
            dict(result.terminal_receipt)
            if result.terminal_receipt is not None
            else None
        ),
        "terminal_receipt_summary": (
            _runtime_receipt_dict(result.terminal_receipt_summary)
            if result.terminal_receipt_summary is not None
            else None
        ),
        "error": _failure_dict(result.error) if result.error is not None else None,
    }
    return value


def _result_response_dict(result: Mapping[str, object]) -> dict[str, object]:
    if result.get("ok") is not True:
        error = result.get("error")
        message = "daemon invocation failed"
        raw_code: str | None = None
        if isinstance(error, Mapping) and isinstance(error.get("message"), str):
            message = error["message"]
        if isinstance(error, Mapping) and isinstance(error.get("code"), str):
            raw_code = error["code"]
        raise SDKError(
            code=canonical_failure_code(raw_code),
            stage="transport",
            retry=RetryHint.UNKNOWN,
            retryable=False,
            message=message,
            details={"runtime_result": dict(result)},
        )
    admission_receipt = result.get("admission_receipt")
    terminal_receipt = result.get("terminal_receipt")
    terminal_state = _terminal_state_name(result.get("terminal_state"))
    response: dict[str, object] = {
        "ok": result.get("ok") is True,
        "state": _terminal_state_code(terminal_state),
        "terminal_state": terminal_state,
        "result_content_type": _string_or_empty(result.get("output_content_type")),
        "result_base64": _string_or_empty(result.get("output_base64")),
        "result_json": result.get("output_json"),
        "elapsed_ms": _non_negative_int(result.get("elapsed_ms")),
        "admission_receipt": (
            dict(admission_receipt) if isinstance(admission_receipt, Mapping) else None
        ),
        "terminal_receipt": (
            dict(terminal_receipt) if isinstance(terminal_receipt, Mapping) else None
        ),
        "sdk_runtime_result": dict(result),
    }
    if result.get("error") is not None:
        response["error"] = result["error"]
    return response


def _terminal_state_name(value: object) -> str:
    if isinstance(value, str) and value:
        return value
    return "Unspecified"


_TERMINAL_STATE_CODES = {
    "unspecified": 0,
    "accepted": 1,
    "admitted": 2,
    "dispatched": 3,
    "running": 4,
    "completed": 5,
    "failed": 6,
    "timed_out": 7,
    "timedout": 7,
    "cancelled": 8,
    "canceled": 8,
}


def _terminal_state_code(value: str) -> int:
    normalized = value.replace("-", "_").lower()
    return _TERMINAL_STATE_CODES.get(normalized, 0)


def _runtime_receipt_dict(receipt: RuntimeReceipt) -> dict[str, object]:
    return {
        "receipt_id": receipt.receipt_id,
        "receipt_ura": receipt.receipt_ura,
        "invocation_id": receipt.invocation_id,
        "receipt_type": receipt.receipt_type,
        "state": receipt.state,
        "index": receipt.index,
        "timestamp_unix_ms": receipt.timestamp_unix_ms,
        "prev_receipt_hash_hex": receipt.prev_receipt_hash_hex,
        "self_hash_hex": receipt.self_hash_hex,
        "cleanup_complete": receipt.cleanup_complete,
        "reason": receipt.reason,
        "child_invocation_id": receipt.child_invocation_id,
        "has_causal_anchor": receipt.has_causal_anchor(),
        "raw": receipt.to_json_dict(),
    }


def _failure_dict(error: InvocationFailure) -> dict[str, object]:
    return {
        "code": error.code,
        "stage": error.stage,
        "message": error.message,
        "retryable": error.retryable,
    }


def _stream_event_dict(event: StreamEvent) -> dict[str, object]:
    return {
        "sequence": event.sequence,
        "kind": event.kind,
        "state": event.state,
        "terminal": event.terminal,
        "content_type": event.payload_content_type,
        "payload_content_type": event.payload_content_type,
        "payload_base64": event.payload_base64,
        "payload_json": event.payload_json,
        "error": event.error,
    }


def _stream_cancel_dict(cancel: StreamCancel) -> dict[str, object]:
    return {
        "stream_id": cancel.stream_id,
        "cancelled": cancel.cancelled,
        "state": cancel.state.value,
        "terminal": cancel.terminal,
    }


def _bidi_frame_dict(frame: BidiFrame) -> dict[str, object]:
    decoded = json.loads(frame.to_json().decode("utf-8"))
    if not isinstance(decoded, dict):
        raise TypeError("bidi frame projection must be an object")
    return decoded


def _bidi_outcome_dict(outcome: BidiOutcome) -> dict[str, object]:
    return {
        "session_id": outcome.session_id,
        "state": outcome.state.value,
        "terminal": outcome.terminal,
        "reason": outcome.reason,
    }


def _stream_error_payload(value: Any) -> Mapping[str, object] | None:
    if (
        isinstance(value, Mapping)
        and set(value) == {"error"}
        and isinstance(value["error"], Mapping)
        and "kind" in value["error"]
    ):
        return value["error"]
    return None


_REMOTE_ERROR_CODES = {
    "CANCELLED": ErrorCode.CANCELLED,
    "DEADLINE_EXCEEDED": ErrorCode.TIMEOUT,
    "UNAVAILABLE": ErrorCode.DAEMON_OFFLINE,
    "INVALID_ARGUMENT": ErrorCode.INVALID_ARGUMENT,
    "RESOURCE_EXHAUSTED": ErrorCode.ADMISSION_DENIED,
    "PERMISSION_DENIED": ErrorCode.PERMISSION_DENIED,
    "INTERNAL": ErrorCode.ABILITY_FAILED,
}


def _remote_wire_error(error: object, *, stage: str = "stream") -> SDKError:
    if not isinstance(error, Mapping):
        return SDKError(
            code=ErrorCode.PROTOCOL_MISMATCH,
            stage=stage,
            retry=RetryHint.UNKNOWN,
            retryable=False,
            message="remote frame error",
            details={"reason": "remote_frame_error", "wire_error": error},
        )
    kind = error.get("kind")
    reason = error.get("reason")
    message = error.get("message")
    kind_text = kind if isinstance(kind, str) else ""
    reason_text = reason if isinstance(reason, str) else ""
    message_text = message if isinstance(message, str) else ""
    code: ErrorCode | str | None = (
        _REMOTE_ERROR_CODES.get(kind_text) if kind_text else ErrorCode.PROTOCOL_MISMATCH
    )
    if code is None:
        code = canonical_failure_code(kind_text)
    return SDKError(
        code=code,
        stage=stage,
        retry=RetryHint.UNKNOWN,
        retryable=False,
        message=message_text or reason_text or kind_text or "remote frame error",
        details={
            "kind": kind_text,
            "reason": reason_text,
            "wire_error": dict(error),
        },
    )


def _json_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def _optional_string(value: object, field_name: str) -> str:
    if value is None:
        return ""
    if not isinstance(value, str):
        raise _invalid_transport(f"{field_name} must be a string or null")
    return value


def _string_or_empty(value: object) -> str:
    return value if isinstance(value, str) else ""


def _non_negative_int(value: object) -> int:
    if isinstance(value, int) and not isinstance(value, bool) and value >= 0:
        return value
    return 0


def _invalid_transport(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.INVALID_ARGUMENT,
        stage="transport",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )


def _missing_required_signer() -> SDKError:
    return SDKError(
        code=ErrorCode.NOT_IMPLEMENTED,
        stage="runtime_signing",
        retry=RetryHint.NEVER,
        retryable=False,
        message=("Signed invocation requires a daemon-authorized SDK Signer"),
        details={"reason": "signing_path_pending"},
    )


def _closed_transport(message: str) -> SDKError:
    return SDKError(
        code=ErrorCode.CANCELLED,
        stage="transport",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
    )
