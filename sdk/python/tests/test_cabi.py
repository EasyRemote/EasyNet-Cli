import ctypes
import json
import unittest

from easynet_sdk import Client, ErrorCode, IdentityClient, SDKError, is_code
from easynet_sdk._cabi import (
    CABIDiscoveryTransport,
    CABIIdentityTransport,
    CLILibrary,
    EXPECTED_ABI_VERSION,
)


class FakeSymbol:
    def __init__(self, func):
        self.func = func
        self.argtypes = None
        self.restype = None

    def __call__(self, *args):
        return self.func(*args)


class FakeRawCABI:
    def __init__(self) -> None:
        self.buffers: dict[int, ctypes.Array[ctypes.c_char]] = {}
        self.last_error_json = b"null"
        self.shutdown_handles: list[int] = []
        self.identity_requests: list[tuple[str, object]] = []
        self.easynet_abi_version = FakeSymbol(lambda: EXPECTED_ABI_VERSION)
        self.easynet_string_free = FakeSymbol(self._free)
        self.easynet_feature_discovery = FakeSymbol(self._feature_discovery)
        self.easynet_last_error_json = FakeSymbol(self._last_error_json)
        self.easynet_error_json = FakeSymbol(self._error_json)
        self.easynet_init = FakeSymbol(self._init)
        self.easynet_shutdown = FakeSymbol(self._shutdown)
        self.easynet_identity_project_ura = FakeSymbol(self._identity_project_ura)
        self.easynet_identity_build_ura = FakeSymbol(self._identity_build_ura)
        self.easynet_identity_project_descriptor_ref = FakeSymbol(
            self._identity_project_descriptor_ref
        )
        self.easynet_identity_build_descriptor_ref = FakeSymbol(
            self._identity_build_descriptor_ref
        )

    def _write(self, out_ptr, payload: bytes) -> int:
        buffer = ctypes.create_string_buffer(payload)
        address = ctypes.addressof(buffer)
        self.buffers[address] = buffer
        out_ptr._obj.value = address
        return 0

    def _free(self, ptr) -> None:
        value = ptr.value if isinstance(ptr, ctypes.c_void_p) else int(ptr)
        self.buffers.pop(value, None)

    def _feature_discovery(self, out_ptr) -> int:
        return self._write(
            out_ptr,
            b'{"abi_version":4,"sdk_version":"0.91.30",'
            b'"profiles":{"directory_identity":"read_model_projection_partial"},'
            b'"symbols":{"directory_identity_projection":true},"axon_pb":true}',
        )

    def _last_error_json(self, out_ptr) -> int:
        return self._write(out_ptr, self.last_error_json)

    def _error_json(self, code, message, out_ptr) -> int:
        return self._write(
            out_ptr,
            json.dumps(
                {
                    "code": "GENERIC" if code else "OK",
                    "stage": "cabi",
                    "message": "",
                    "retry": "never",
                    "source": "cabi",
                    "details": {},
                },
                separators=(",", ":"),
            ).encode("utf-8"),
        )

    def _init(self, control_path, out_handle) -> int:
        out_handle._obj.value = 42
        return 0

    def _shutdown(self, handle) -> int:
        self.shutdown_handles.append(int(handle.value))
        return 0

    def _identity_project_ura(self, handle, raw, out_ptr) -> int:
        self.identity_requests.append(("project_ura", raw.value.decode("utf-8")))
        return self._write(
            out_ptr,
            b'{"kind":"ability","valid":true,'
            b'"ura":"easynet:///r/example/ability/device.dev-a.observe.health",'
            b'"profile":"easynet-strict-v2",'
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a"},'
            b'"metadata":{"grammar_owner":"axon"}}',
        )

    def _identity_build_ura(self, handle, raw, out_ptr) -> int:
        request = json.loads(raw.value.decode("utf-8"))
        self.identity_requests.append(("build_ura", request))
        return self._identity_project_ura(
            handle,
            ctypes.c_char_p(b"easynet:///r/example/ability/device.dev-a.observe.health"),
            out_ptr,
        )

    def _identity_project_descriptor_ref(self, handle, raw, out_ptr) -> int:
        self.identity_requests.append(
            ("project_descriptor_ref", raw.value.decode("utf-8"))
        )
        return self._write(out_ptr, DESCRIPTOR_PROJECTION)

    def _identity_build_descriptor_ref(self, handle, raw, out_ptr) -> int:
        request = json.loads(raw.value.decode("utf-8"))
        self.identity_requests.append(("build_descriptor_ref", request))
        return self._write(out_ptr, DESCRIPTOR_PROJECTION)


DESCRIPTOR_PROJECTION = (
    b'{"kind":"descriptor_ref","valid":true,'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",'
    b'"ability_ura":"easynet:///r/example/ability/device.dev-a.observe.health",'
    b'"descriptor_version":"1.0.0","profile":"easynet-strict-v2",'
    b'"components":{"owner_ura":"easynet:///r/example/device/dev-a"},'
    b'"metadata":{"grammar_owner":"axon"}}'
)


class CABITransportTests(unittest.TestCase):
    def test_feature_discovery_uses_cabi_v4(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = Client(CABIDiscoveryTransport(lib))

        features = client.require_abi(EXPECTED_ABI_VERSION)

        self.assertTrue(features.axon_pb)
        self.assertTrue(features.symbols["directory_identity_projection"])

    def test_identity_transport_drives_addressing_helpers(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        transport = CABIIdentityTransport(lib, handle=7)
        client = IdentityClient(transport)

        ability_ura = client.owner_ability_ura(
            "easynet:///r/example/device/dev-a", "observe.health"
        )
        owner_ura = client.owner_ura_for_ability(ability_ura)
        descriptor_ref = client.canonical_ability_descriptor_ref(ability_ura, "1.0.0")

        self.assertEqual(
            ability_ura, "easynet:///r/example/ability/device.dev-a.observe.health"
        )
        self.assertEqual(owner_ura, "easynet:///r/example/device/dev-a")
        self.assertEqual(
            descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )
        self.assertEqual(
            raw.identity_requests,
            [
                (
                    "build_ura",
                    {
                        "kind": "ability",
                        "owner_ura": "easynet:///r/example/device/dev-a",
                        "ability_name": "observe.health",
                    },
                ),
                (
                    "project_ura",
                    "easynet:///r/example/ability/device.dev-a.observe.health",
                ),
                (
                    "project_ura",
                    "easynet:///r/example/ability/device.dev-a.observe.health",
                ),
                (
                    "build_descriptor_ref",
                    {
                        "ability_ura": "easynet:///r/example/ability/device.dev-a.observe.health",
                        "descriptor_version": "1.0.0",
                    },
                ),
            ],
        )
        self.assertEqual(raw.buffers, {})

    def test_owned_identity_transport_closes_handle_once(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        handle = lib.init("")
        transport = CABIIdentityTransport(lib, handle=handle, owns_handle=True)

        transport.close()
        transport.close()

        self.assertEqual(raw.shutdown_handles, [42])

    def test_cabi_error_json_projects_sdk_error(self) -> None:
        raw = FakeRawCABI()
        raw.last_error_json = (
            b'{"code":"INVALID_ARGUMENT","stage":"cabi","message":"bad input",'
            b'"retry":"never","source":"cabi","details":{}}'
        )
        lib = CLILibrary(raw)
        raw.easynet_feature_discovery = FakeSymbol(lambda out_ptr: 11)

        with self.assertRaises(SDKError) as caught:
            lib.feature_discovery()

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(raw.buffers, {})


if __name__ == "__main__":
    unittest.main()
