"""Provider-neutral runtime-event lifecycle model."""

from __future__ import annotations

from enum import StrEnum


class RuntimeEventStreamState(StrEnum):
    LIVE = "Live"
    TERMINAL = "Terminal"
    FAILED = "Failed"


def transition_runtime_event_stream(
    current: RuntimeEventStreamState,
    next_state: RuntimeEventStreamState,
) -> None:
    if current is RuntimeEventStreamState.LIVE:
        if next_state in RuntimeEventStreamState:
            return
    elif current in {
        RuntimeEventStreamState.TERMINAL,
        RuntimeEventStreamState.FAILED,
    } and next_state is current:
        return
    raise ValueError(
        f"runtime event stream cannot transition from {current.value!r} "
        f"to {next_state.value!r}"
    )


def validate_runtime_event_page_state(
    state: RuntimeEventStreamState, *, terminal: bool
) -> None:
    expected = state in {
        RuntimeEventStreamState.TERMINAL,
        RuntimeEventStreamState.FAILED,
    }
    if terminal is not expected:
        raise ValueError(
            f"runtime event terminal flag does not match state {state.value!r}"
        )
    transition_runtime_event_stream(RuntimeEventStreamState.LIVE, state)
