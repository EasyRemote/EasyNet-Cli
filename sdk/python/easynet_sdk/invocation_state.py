"""Normative generic Invocation lifecycle states."""

from enum import IntEnum


class InvocationLifecycleState(IntEnum):
    UNSPECIFIED = 0
    ACCEPTED = 1
    ADMITTED = 2
    DISPATCHED = 3
    RUNNING = 4
    COMPLETED = 5
    FAILED = 6
    TIMED_OUT = 7
    CANCELLED = 8

    @property
    def is_terminal(self) -> bool:
        return self in {
            self.COMPLETED,
            self.FAILED,
            self.TIMED_OUT,
            self.CANCELLED,
        }
