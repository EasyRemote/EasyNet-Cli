import pytest

from easynet_sdk import (
    AddressingClient,
    AxonAddressingTransport,
    InvocationHandle,
    RuntimeClient,
    RuntimeAbilityClient,
    RuntimeAbilityEventSubscriptionProvider,
    RuntimeCallContext,
    RuntimeEventClient,
    RuntimeEventCursor,
    RuntimeEventReadRequest,
    RuntimeEventStreamKind,
    RuntimeEventStreamState,
    RuntimeEventSubscriptionClient,
    RuntimeEventSubscriptionCursor,
    RuntimeEventSubscriptionRequest,
    RuntimeHandleEventProvider,
    SDKError,
    runtime_event_subscription_ability,
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


def test_runtime_event_subscription_provider_builds_device_draft() -> None:
    client = RuntimeEventSubscriptionClient(
        RuntimeAbilityEventSubscriptionProvider(
            RuntimeAbilityClient(
                RuntimeClient(MemoryRuntimeTransport()),
                AddressingClient(AxonAddressingTransport()),
            )
        )
    )

    draft = client.build(
        RuntimeEventSubscriptionRequest(
            call=_call(),
            stream=RuntimeEventStreamKind.DEVICE,
            realm="example",
            owner_ura="easynet:///r/example/user/alice",
            device_ura="easynet:///r/example/device/laptop",
            resume_cursor=RuntimeEventSubscriptionCursor(stream="device", sequence=42),
            heartbeat_interval_ms=30000,
        )
    )

    assert draft.descriptor_ref == (
        "easynet:///r/example/ability/hub.events.device.subscribe@1.0.0"
    )
    assert draft.metadata["sdk_profile"] == "runtime_events"
    assert draft.metadata["system_ability"] == "events.device.subscribe"
    assert draft.args["stream"] == "device"
    assert draft.args["daemon_ability"] == "events.device.subscribe"
    assert draft.args["realm"] == "example"
    assert draft.args["owner_ura"] == "easynet:///r/example/user/alice"
    assert draft.args["device_ura"] == "easynet:///r/example/device/laptop"
    assert draft.args["heartbeat_interval_ms"] == 30000
    assert draft.args["resume_cursor"] == "device:42"


def test_runtime_event_subscription_provider_builds_session_draft_with_since_sequence() -> None:
    client = RuntimeEventSubscriptionClient(
        RuntimeAbilityEventSubscriptionProvider(
            RuntimeAbilityClient(
                RuntimeClient(MemoryRuntimeTransport()),
                AddressingClient(AxonAddressingTransport()),
            )
        )
    )

    draft = client.build(
        RuntimeEventSubscriptionRequest(
            call=_call(),
            stream=RuntimeEventStreamKind.SESSION,
            session_id="session-1",
            resume_cursor=RuntimeEventSubscriptionCursor(stream="session", sequence=42),
        )
    )

    assert draft.descriptor_ref == (
        "easynet:///r/example/ability/hub.session.attach@1.0.0"
    )
    assert draft.metadata["system_ability"] == "session.attach"
    assert "stream" not in draft.args
    assert "daemon_ability" not in draft.args
    assert draft.args["session_id"] == "session-1"
    assert draft.args["since_seq"] == 42


def test_runtime_event_subscription_ability_rejects_unsupported_stream() -> None:
    with pytest.raises(SDKError, match="unsupported runtime event stream"):
        runtime_event_subscription_ability("unknown")  # type: ignore[arg-type]


def _call() -> RuntimeCallContext:
    return RuntimeCallContext(
        caller_ura="easynet:///r/example/agent/alice.client",
        callee_ura="easynet:///r/example/hub",
        subject_ura="easynet:///r/example/user/alice",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        metadata={"request_id": "events-1"},
    )
