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
        """Decode the finite canonical lifecycle wire vocabulary."""

        states = {
            "unspecified": cls.UNSPECIFIED,
            "Unspecified": cls.UNSPECIFIED,
            "UNSPECIFIED": cls.UNSPECIFIED,
            "accepted": cls.ACCEPTED,
            "Accepted": cls.ACCEPTED,
            "ACCEPTED": cls.ACCEPTED,
            "admitted": cls.ADMITTED,
            "Admitted": cls.ADMITTED,
            "ADMITTED": cls.ADMITTED,
            "dispatched": cls.DISPATCHED,
            "Dispatched": cls.DISPATCHED,
            "DISPATCHED": cls.DISPATCHED,
            "running": cls.RUNNING,
            "Running": cls.RUNNING,
            "RUNNING": cls.RUNNING,
            "completed": cls.COMPLETED,
            "Completed": cls.COMPLETED,
            "COMPLETED": cls.COMPLETED,
            "failed": cls.FAILED,
            "Failed": cls.FAILED,
            "FAILED": cls.FAILED,
            "timed_out": cls.TIMED_OUT,
            "TimedOut": cls.TIMED_OUT,
            "TIMED_OUT": cls.TIMED_OUT,
            "cancelled": cls.CANCELLED,
            "Cancelled": cls.CANCELLED,
            "CANCELLED": cls.CANCELLED,
        }
        if not isinstance(value, str) or value not in states:
            raise ValueError(f"unknown invocation lifecycle state: {value!r}")
        return states[value]
