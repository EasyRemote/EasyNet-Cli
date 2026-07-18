"""Local transport-carrier lifecycle shared by runtime facades."""

from enum import Enum, auto

from .errors import ErrorCode, SDKError


class CarrierState(Enum):
    OPEN = auto()
    CLOSING = auto()
    CLOSED = auto()
    FAILED = auto()

    @property
    def is_open(self) -> bool:
        return self is CarrierState.OPEN


def is_local_carrier_interruption(error: BaseException) -> bool:
    return isinstance(error, TimeoutError) or (
        isinstance(error, SDKError)
        and error.code in {ErrorCode.TIMEOUT, ErrorCode.CANCELLED}
    )
