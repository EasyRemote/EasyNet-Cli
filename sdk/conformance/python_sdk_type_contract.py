"""Strict static contract for canonical Python SDK runtime models."""

from typing import assert_type

from easynet_sdk.directory import (
    DirectoryCursor,
    DirectoryResolveKind,
    DirectoryResolveRequest,
)
from easynet_sdk.runtime_ability import RuntimeCallContext
from easynet_sdk.runtime_events import (
    RuntimeEventCursor,
    RuntimeEventPage,
    RuntimeEventStreamState,
)


def verify_runtime_model_contracts() -> None:
    call = RuntimeCallContext(
        caller_ura="easynet:///r/example/user/alice",
        callee_ura="easynet:///r/example/user/alice",
        subject_ura="easynet:///r/example/user/alice",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
    )
    assert_type(RuntimeEventCursor(sequence=1), RuntimeEventCursor)
    assert_type(
        RuntimeEventPage(
            events=(),
            cursor=RuntimeEventCursor(sequence=1),
            state=RuntimeEventStreamState.TERMINAL,
            terminal=True,
            limit=1,
        ),
        RuntimeEventPage,
    )
    assert_type(DirectoryCursor.at(1), DirectoryCursor)
    assert_type(
        DirectoryResolveRequest(
            call=call,
            query_ura="easynet:///r/example/user/alice",
            kind=DirectoryResolveKind.CANONICAL_IDENTITY,
        ),
        DirectoryResolveRequest,
    )
