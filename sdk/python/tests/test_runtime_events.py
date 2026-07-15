import pytest

from easynet_sdk import (
    InvocationHandle,
    RuntimeClient,
    RuntimeEventClient,
    RuntimeEventCursor,
    RuntimeEventPage,
    RuntimeEventReadRequest,
    RuntimeEventStreamState,
    RuntimeHandleEventProvider,
    SDKError,
)

from test_runtime import MemoryRuntimeTransport


def test_runtime_event_client_reads_bounded_typed_page() -> None:
    runtime = RuntimeClient(MemoryRuntimeTransport())
    provider = RuntimeHandleEventProvider(runtime)
    client = RuntimeEventClient(provider)
    handle = InvocationHandle._from_runtime_json(
        b'{"handle_id":7,"state":"Submitted","terminal":false,"events":[],"result":null}'
    )

    page = client.read(
        RuntimeEventReadRequest(
            handle=handle,
            cursor=RuntimeEventCursor(sequence=1),
            limit=1,
        )
    )

    assert len(page.events) == 1
    assert page.events[0].sequence == 2
    assert page.cursor.sequence == 2
    assert page.state == RuntimeEventStreamState.TERMINAL
    assert page.terminal is True


def test_runtime_event_client_treats_failed_invocation_as_terminal_feed() -> None:
    class FailedInvocationTransport(MemoryRuntimeTransport):
        def handle_events(self, control) -> bytes:
            return (
                b'{"handle_id":7,"state":"Failed","terminal":true,'
                b'"events":[{"sequence":1,"kind":"submitted",'
                b'"state":"Submitted","terminal":false},{"sequence":2,'
                b'"kind":"failed","state":"Failed","terminal":true}],'
                b'"result":{"ok":false}}'
            )

    provider = RuntimeHandleEventProvider(RuntimeClient(FailedInvocationTransport()))
    handle = InvocationHandle._from_runtime_json(
        b'{"handle_id":7,"state":"Submitted","terminal":false,"events":[],"result":null}'
    )

    page = provider.read_events(RuntimeEventReadRequest(handle=handle))

    assert page.state == RuntimeEventStreamState.TERMINAL
    assert page.terminal is True
    assert page.events[-1].state == "Failed"


def test_runtime_event_client_rejects_unbounded_limit() -> None:
    runtime = RuntimeClient(MemoryRuntimeTransport())
    provider = RuntimeHandleEventProvider(runtime)
    handle = InvocationHandle._from_runtime_json(
        b'{"handle_id":7,"state":"Submitted","terminal":false,"events":[],"result":null}'
    )

    with pytest.raises(SDKError, match="runtime event page limit exceeds maximum"):
        provider.read_events(RuntimeEventReadRequest(handle=handle, limit=501))


def test_runtime_event_page_rejects_incoherent_terminal_flag() -> None:
    with pytest.raises(SDKError, match="terminal flag does not match"):
        RuntimeEventPage(
            events=(),
            cursor=RuntimeEventCursor(),
            state=RuntimeEventStreamState.LIVE,
            terminal=True,
            limit=1,
        )
