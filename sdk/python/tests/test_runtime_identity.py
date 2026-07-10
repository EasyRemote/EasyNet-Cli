import base64
import json
import os
import socket
import struct
import tempfile
import threading
import unittest

from easynet_sdk.runtime_identity import (
    ensure_runtime_signing_identity,
    load_runtime_signing_identity,
)


class RuntimeIdentityTests(unittest.TestCase):
    def test_load_and_sign_use_daemon_keyring_protocol(self) -> None:
        public_key = bytes(range(32))
        signature = bytes(range(64))
        requests: list[dict[str, object]] = []
        with _keyring_server(
            [
                {"result": "public_key", "public_key_b64": base64.b64encode(public_key).decode()},
                {"result": "signature", "signature_b64": base64.b64encode(signature).decode()},
            ],
            requests,
        ) as socket_path:
            identity = load_runtime_signing_identity("easynet:///r/acme/hub", socket_path=socket_path)
            self.assertEqual(identity.public_key, public_key)
            self.assertEqual(identity.sign_canonical(b"canonical"), signature)
        self.assertEqual(requests[0], {"method": "derive_pubkey", "self_ura": "easynet:///r/acme/hub"})
        self.assertEqual(requests[1]["method"], "sign")
        self.assertNotIn("private_key_seed", requests[1])
        self.assertNotIn("vault_path", requests[1])

    def test_ensure_delegates_key_generation_to_daemon(self) -> None:
        public_key = bytes(range(32))
        requests: list[dict[str, object]] = []
        with _keyring_server(
            [{"result": "public_key", "public_key_b64": base64.b64encode(public_key).decode()}],
            requests,
        ) as socket_path:
            identity = ensure_runtime_signing_identity(
                "easynet:///r/acme/hub",
                role_overlays=("easynet:///r/acme/device/dev-a",),
                socket_path=socket_path,
            )
        self.assertEqual(identity.public_key, public_key)
        self.assertEqual(requests[0]["method"], "ensure")
        self.assertNotIn("seed_hex", requests[0])


class _keyring_server:
    def __init__(self, responses: list[dict[str, object]], requests: list[dict[str, object]]) -> None:
        self._responses = responses
        self._requests = requests
        self._directory = tempfile.mkdtemp(prefix="ek-")
        self._path = os.path.join(self._directory, "keyring.sock")
        self._listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._thread: threading.Thread | None = None

    def __enter__(self) -> str:
        self._listener.bind(self._path)
        self._listener.listen()
        self._thread = threading.Thread(target=self._serve, daemon=True)
        self._thread.start()
        return self._path

    def __exit__(self, *_: object) -> None:
        self._listener.close()
        if self._thread:
            self._thread.join(timeout=1)
        try:
            os.unlink(self._path)
        except FileNotFoundError:
            pass
        os.rmdir(self._directory)

    def _serve(self) -> None:
        for response in self._responses:
            connection, _ = self._listener.accept()
            with connection:
                length = struct.unpack(">I", _read_exact(connection, 4))[0]
                self._requests.append(json.loads(_read_exact(connection, length).decode()))
                encoded = json.dumps(response, separators=(",", ":")).encode()
                connection.sendall(struct.pack(">I", len(encoded)) + encoded)


def _read_exact(connection: socket.socket, count: int) -> bytes:
    data = bytearray()
    while len(data) < count:
        data.extend(connection.recv(count - len(data)))
    return bytes(data)
