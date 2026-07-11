import pytest

from easynet_sdk import (
    InvocationHandle,
    RuntimeClient,
    RuntimeEventClient,
    RuntimeEventCursor,
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
    handle = InvocationHandle.from_json(
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


def test_runtime_event_client_rejects_unbounded_limit() -> None:
    runtime = RuntimeClient(MemoryRuntimeTransport())
    provider = RuntimeHandleEventProvider(runtime)
    handle = InvocationHandle.from_json(
        b'{"handle_id":7,"state":"Submitted","terminal":false,"events":[],"result":null}'
    )

    with pytest.raises(SDKError, match="runtime event page limit exceeds maximum"):
        provider.read_events(RuntimeEventReadRequest(handle=handle, limit=501))
