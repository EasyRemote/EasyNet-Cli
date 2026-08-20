"""Reusable fake runtime key-service endpoint for facade conformance tests."""

from __future__ import annotations

import json
import os
import socket
import struct
import tempfile
import threading
import time
from collections.abc import Sequence
from dataclasses import dataclass


@dataclass(frozen=True)
class KeyServiceResponsePlan:
    response: dict[str, object] | None
    chunk_size: int = 0
    chunk_delay_seconds: float = 0.0


class KeyServiceServer:
    def __init__(
        self,
        responses: Sequence[dict[str, object] | KeyServiceResponsePlan],
    ) -> None:
        self.requests: list[dict[str, object]] = []
        self._responses = responses
        self._directory = tempfile.mkdtemp(prefix="key-service-")
        self._path = os.path.join(self._directory, "keyring.sock")
        self._listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._thread: threading.Thread | None = None
        self._error: BaseException | None = None
        self._stopping = threading.Event()

    def __enter__(self) -> "KeyServiceServer":
        self._listener.bind(self._path)
        self._listener.listen()
        self._thread = threading.Thread(target=self._serve, daemon=True)
        self._thread.start()
        return self

    def __exit__(self, *_: object) -> None:
        self._stopping.set()
        self._listener.close()
        if self._thread:
            self._thread.join(timeout=1)
        try:
            os.unlink(self._path)
        except FileNotFoundError:
            pass
        os.rmdir(self._directory)
        if self._error is not None:
            raise self._error

    @property
    def socket_path(self) -> str:
        return self._path

    def _serve(self) -> None:
        try:
            for configured in self._responses:
                connection, _ = self._listener.accept()
                with connection:
                    length = struct.unpack(">I", _read_exact(connection, 4))[0]
                    decoded = json.loads(_read_exact(connection, length).decode())
                    if not isinstance(decoded, dict):
                        raise AssertionError("key-service request must be an object")
                    self.requests.append(decoded)
                    plan = (
                        configured
                        if isinstance(configured, KeyServiceResponsePlan)
                        else KeyServiceResponsePlan(configured)
                    )
                    if plan.response is None:
                        continue
                    encoded = json.dumps(plan.response, separators=(",", ":")).encode()
                    frame = struct.pack(">I", len(encoded)) + encoded
                    if plan.chunk_size <= 0:
                        connection.sendall(frame)
                        continue
                    for offset in range(0, len(frame), plan.chunk_size):
                        connection.sendall(frame[offset : offset + plan.chunk_size])
                        if plan.chunk_delay_seconds > 0:
                            time.sleep(plan.chunk_delay_seconds)
        except (BrokenPipeError, ConnectionResetError):
            return
        except OSError as exc:
            if self._stopping.is_set():
                return
            self._error = exc
        except BaseException as exc:
            self._error = exc


def _read_exact(connection: socket.socket, count: int) -> bytes:
    data = bytearray()
    while len(data) < count:
        chunk = connection.recv(count - len(data))
        if not chunk:
            raise OSError("unexpected EOF from key-service client")
        data.extend(chunk)
    return bytes(data)
