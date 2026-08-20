import json


class MemoryAddressingTransport:
    def __init__(self) -> None:
        self.descriptor_json = b"{}"
        self.identity_json = b"{}"
        self.identity_jsons: list[bytes] = []
        self.expected_identity_ura: str | None = None
        self.seen_request: dict[str, object] | None = None
        self.seen_requests: list[dict[str, object]] = []
        self.close_calls = 0

    def _record(self, request_json: bytes) -> None:
        self.seen_request = json.loads(request_json.decode("utf-8"))
        self.seen_requests.append(self.seen_request)

    def project_descriptor_ref(self, request_json: bytes) -> bytes:
        self._record(request_json)
        return self.descriptor_json

    def build_descriptor_ref(self, request_json: bytes) -> bytes:
        self._record(request_json)
        return self.descriptor_json

    def project_identity(self, request_json: bytes) -> bytes:
        self._record(request_json)
        if (
            self.expected_identity_ura is not None
            and self.seen_request is not None
            and self.seen_request.get("ura") != self.expected_identity_ura
        ):
            raise ValueError("projected URA is not the configured ability identity")
        return self._identity_json()

    def build_ura(self, request_json: bytes) -> bytes:
        self._record(request_json)
        return self._identity_json()

    def close(self) -> None:
        self.close_calls += 1

    def _identity_json(self) -> bytes:
        return self.identity_jsons.pop(0) if self.identity_jsons else self.identity_json
