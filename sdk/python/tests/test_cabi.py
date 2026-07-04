import ctypes
import json
import unittest

from easynet_sdk import (
    Client,
    ErrorCode,
    HealthClient,
    IdentityClient,
    InvocationSignature,
    PrepareOptions,
    RuntimeClient,
    SDKError,
    is_code,
)
from easynet_sdk._cabi import (
    CABIDiscoveryTransport,
    CABIIdentityTransport,
    CABIRuntimeTransport,
    CLILibrary,
    EXPECTED_ABI_VERSION,
)

from test_runtime import complete_draft
from test_signing import PREPARED_FIXTURE


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
        self.runtime_requests: list[tuple[str, object]] = []
        self.prepared_frees: list[int] = []
        self.signed_frees: list[int] = []
        self.handle_frees: list[tuple[int, int]] = []
        self.prepare_payload = PREPARED_FIXTURE
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
        self.easynet_runtime_health = FakeSymbol(self._runtime_health)
        self.easynet_invocation_invoke = FakeSymbol(self._invocation_invoke)
        self.easynet_invocation_prepare = FakeSymbol(self._invocation_prepare)
        self.easynet_invocation_sign_prepared = FakeSymbol(
            self._invocation_sign_prepared
        )
        self.easynet_invocation_submit_signed_handle = FakeSymbol(
            self._invocation_submit_signed_handle
        )
        self.easynet_invocation_handle_await = FakeSymbol(
            self._invocation_handle_await
        )
        self.easynet_invocation_handle_cancel = FakeSymbol(
            self._invocation_handle_cancel
        )
        self.easynet_invocation_handle_events = FakeSymbol(
            self._invocation_handle_events
        )
        self.easynet_invocation_handle_free = FakeSymbol(
            self._invocation_handle_free
        )
        self.easynet_prepared_invocation_free = FakeSymbol(
            self._prepared_invocation_free
        )
        self.easynet_signed_invocation_free = FakeSymbol(self._signed_invocation_free)

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

    def _runtime_health(self, handle, out_ptr) -> int:
        self.runtime_requests.append(("health", int(handle.value)))
        return self._write(
            out_ptr,
            b'{"api_ready":true,"daemon_ready":true,"invocation_ready":true,'
            b'"directory_ready":true,"trust_ready":true,"runtime_ready":true,'
            b'"version":"0.91.30","abi_version":4,"mismatch":null,'
            b'"diagnostics":[]}',
        )

    def _invocation_invoke(self, handle, raw, out_ptr) -> int:
        draft = json.loads(raw.value.decode("utf-8"))
        self.runtime_requests.append(("invoke", draft))
        return self._write(
            out_ptr,
            json.dumps(
                {
                    "ok": True,
                    "tuple": draft,
                    "terminal_state": "Completed",
                    "output_content_type": "application/json",
                    "output_base64": "e30=",
                    "output_json": {},
                    "elapsed_ms": 7,
                    "receipt": None,
                    "error": None,
                },
                separators=(",", ":"),
                sort_keys=True,
            ).encode("utf-8"),
        )

    def _invocation_prepare(
        self, handle, raw, options, out_prepared_id, out_ptr
    ) -> int:
        self.runtime_requests.append(
            (
                "prepare",
                {
                    "handle": int(handle.value),
                    "draft": json.loads(raw.value.decode("utf-8")),
                    "options": json.loads(options.value.decode("utf-8")),
                },
            )
        )
        out_prepared_id._obj.value = 101
        return self._write(out_ptr, self.prepare_payload)

    def _invocation_sign_prepared(
        self, prepared_id, signature_json, out_signed_id, out_ptr
    ) -> int:
        self.runtime_requests.append(
            (
                "sign_prepared",
                {
                    "prepared_id": int(prepared_id.value),
                    "signature": json.loads(signature_json.value.decode("utf-8")),
                },
            )
        )
        out_signed_id._obj.value = 202
        return self._write(
            out_ptr,
            b'{"signer_id":"caller-key","prepared":{"prepared_id":'
            b'"prepared-example-1","request_id":"","descriptor_ref":'
            b'"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"},'
            b'"signature":{"algorithm":"ed25519","signature_base64":'
            b'"c2lnbmF0dXJl","key_id_hint":"caller-key"}}',
        )

    def _invocation_submit_signed_handle(
        self, handle, signed_id, out_invocation_handle_id, out_ptr
    ) -> int:
        self.runtime_requests.append(
            (
                "submit_signed_handle",
                {"handle": int(handle.value), "signed_id": int(signed_id.value)},
            )
        )
        out_invocation_handle_id._obj.value = 303
        return self._write(
            out_ptr,
            b'{"handle_id":303,"state":"Submitted","terminal":false,'
            b'"events":[{"sequence":1,"kind":"submitted","state":"Submitted",'
            b'"terminal":false}],"result":null}',
        )

    def _invocation_handle_await(self, handle, invocation_handle_id, out_ptr) -> int:
        draft = complete_draft().to_json_dict()
        self.runtime_requests.append(
            (
                "await",
                {
                    "handle": int(handle.value),
                    "handle_id": int(invocation_handle_id.value),
                },
            )
        )
        return self._write(
            out_ptr,
            json.dumps(
                {
                    "ok": True,
                    "tuple": draft,
                    "terminal_state": "Completed",
                    "output_content_type": "application/json",
                    "output_base64": "e30=",
                    "output_json": {},
                    "elapsed_ms": 9,
                    "receipt": None,
                    "error": None,
                },
                separators=(",", ":"),
                sort_keys=True,
            ).encode("utf-8"),
        )

    def _invocation_handle_cancel(
        self, handle, invocation_handle_id, reason, out_ptr
    ) -> int:
        self.runtime_requests.append(
            (
                "cancel",
                {
                    "handle": int(handle.value),
                    "handle_id": int(invocation_handle_id.value),
                    "reason": reason.value.decode("utf-8") if reason.value else "",
                },
            )
        )
        return self._write(
            out_ptr,
            b'{"handle_id":303,"cancelled":true,'
            b'"state":"Cancelled","terminal":true}',
        )

    def _invocation_handle_events(self, handle, invocation_handle_id, out_ptr) -> int:
        self.runtime_requests.append(
            (
                "events",
                {
                    "handle": int(handle.value),
                    "handle_id": int(invocation_handle_id.value),
                },
            )
        )
        return self._write(
            out_ptr,
            b'{"handle_id":303,"state":"Cancelled","terminal":true,'
            b'"events":[{"sequence":1,"kind":"submitted","state":"Submitted",'
            b'"terminal":false},{"sequence":2,"kind":"cancelled",'
            b'"state":"Cancelled","terminal":true,"reason":"client stop"}],'
            b'"result":null}',
        )

    def _invocation_handle_free(self, handle, invocation_handle_id) -> int:
        self.handle_frees.append((int(handle.value), int(invocation_handle_id.value)))
        return 0

    def _prepared_invocation_free(self, prepared_id) -> int:
        self.prepared_frees.append(int(prepared_id.value))
        return 0

    def _signed_invocation_free(self, signed_id) -> int:
        self.signed_frees.append(int(signed_id.value))
        return 0


DESCRIPTOR_PROJECTION = (
    b'{"kind":"descriptor_ref","valid":true,'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",'
    b'"ability_ura":"easynet:///r/example/ability/device.dev-a.observe.health",'
    b'"descriptor_version":"1.0.0","profile":"easynet-strict-v2",'
    b'"components":{"owner_ura":"easynet:///r/example/device/dev-a"},'
    b'"metadata":{"grammar_owner":"axon"}}'
)

CURRENT_ABI_PREPARED = b"""{
  "request_id": "req-current-1",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
  "descriptor_hash_hex": "aa",
  "schema_hash_hex": "bb",
  "canonical_hash_hex": "cc",
  "expires_at_unix_ms": 1783000000000,
  "tuple": {
    "caller_ura": "easynet:///r/example/agent/alice.sdk",
    "callee_ura": "easynet:///r/example/device/dev-a",
    "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
    "subject_ura": "easynet:///r/example/device/dev-a",
    "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
    "causal_context": {"form": "none"},
    "args": {},
    "content_type": "application/json"
  },
  "signing_material": {
    "canonical_bytes_base64": "ZXhhbXBsZQ==",
    "args_digest_hex": "00",
    "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
    "signed_fields": ["caller_ura", "callee_ura"],
    "signer_policy": {
      "mode": "caller_signing",
      "signer_id": "browser-key",
      "policy_ref": "policy/local",
      "expires_at_unix_ms": 1783000000000
    },
    "expires_at_unix_ms": 1783000000000
  }
}"""


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

    def test_runtime_transport_drives_health_and_unary_invoke(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        transport = CABIRuntimeTransport(lib, handle=7)

        health = HealthClient(transport).runtime_health()
        result = RuntimeClient(transport).invoke(complete_draft())

        self.assertTrue(health.ready())
        self.assertTrue(result.ok)
        self.assertEqual(result.terminal_state, "Completed")
        self.assertEqual(
            result.tuple.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )
        self.assertEqual(raw.runtime_requests[0], ("health", 7))
        self.assertEqual(raw.buffers, {})

    def test_runtime_transport_prepare_sign_submit_choreography(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = RuntimeClient(CABIRuntimeTransport(lib, handle=7))

        prepared, _ = client.prepare(
            complete_draft(), PrepareOptions(expires_in_ms=60000)
        )
        signed = prepared.sign_with_caller_signature(
            InvocationSignature(
                algorithm="ed25519",
                signature_base64="c2lnbmF0dXJl",
                key_id_hint="caller-key",
            )
        )
        handle = client.submit_signed(signed)
        result = client.await_result(handle)
        cancel = client.cancel(handle, "client stop")
        events = client.events(handle)
        client.close_handle(handle)

        self.assertEqual(handle.handle_id, 303)
        self.assertTrue(result.ok)
        self.assertTrue(cancel.cancelled)
        self.assertTrue(events.terminal)
        self.assertEqual(raw.handle_frees, [(7, 303)])
        self.assertEqual(
            [kind for kind, _ in raw.runtime_requests if kind != "health"],
            [
                "prepare",
                "sign_prepared",
                "submit_signed_handle",
                "await",
                "cancel",
                "events",
            ],
        )
        self.assertEqual(raw.buffers, {})

    def test_runtime_transport_accepts_current_abi_request_id_prepare_shape(
        self,
    ) -> None:
        raw = FakeRawCABI()
        raw.prepare_payload = CURRENT_ABI_PREPARED
        lib = CLILibrary(raw)
        client = RuntimeClient(CABIRuntimeTransport(lib, handle=7))

        prepared, _ = client.prepare(complete_draft())
        signed = prepared.sign_with_caller_signature(
            InvocationSignature(
                algorithm="ed25519",
                signature_base64="c2lnbmF0dXJl",
                key_id_hint="caller-key",
            )
        )
        handle = client.submit_signed(signed)

        self.assertEqual(prepared.request_id, "req-current-1")
        self.assertEqual(prepared.prepared_id, "")
        self.assertEqual(handle.handle_id, 303)
        self.assertEqual(raw.prepared_frees, [])

    def test_runtime_transport_rejects_foreign_signed_dto(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        transport = CABIRuntimeTransport(lib, handle=7)
        client = RuntimeClient(transport)

        from test_runtime import signed_fixture

        with self.assertRaises(SDKError) as caught:
            client.submit_signed(signed_fixture())

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_HANDLE))

    def test_runtime_transport_stream_and_bidi_are_explicitly_not_implemented(
        self,
    ) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = RuntimeClient(CABIRuntimeTransport(lib, handle=7))

        with self.assertRaises(SDKError) as caught_stream:
            client.invoke_stream(complete_draft())
        with self.assertRaises(SDKError) as caught_bidi:
            client.open_bidi(complete_draft(), ())

        self.assertTrue(is_code(caught_stream.exception, ErrorCode.NOT_IMPLEMENTED))
        self.assertTrue(is_code(caught_bidi.exception, ErrorCode.NOT_IMPLEMENTED))

    def test_owned_runtime_transport_frees_prepared_handles_on_close(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        handle = lib.init("")
        transport = CABIRuntimeTransport(lib, handle=handle, owns_handle=True)

        transport.prepare(
            complete_draft().to_json().encode("utf-8"),
            b'{"expires_in_ms":60000}',
        )
        transport.close()
        transport.close()

        self.assertEqual(raw.prepared_frees, [101])
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
