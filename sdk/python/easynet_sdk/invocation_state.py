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

    @classmethod
    def from_wire_name(cls, value: str) -> "InvocationLifecycleState":
        """Decode the finite canonical lifecycle carrier vocabulary."""

        states = {
            "Unspecified": cls.UNSPECIFIED,
            "Accepted": cls.ACCEPTED,
            "Admitted": cls.ADMITTED,
            "Dispatched": cls.DISPATCHED,
            "Running": cls.RUNNING,
            "Completed": cls.COMPLETED,
            "Failed": cls.FAILED,
            "TimedOut": cls.TIMED_OUT,
            "Cancelled": cls.CANCELLED,
        }
        if not isinstance(value, str) or value not in states:
            raise ValueError(f"unknown invocation lifecycle state: {value!r}")
        return states[value]
