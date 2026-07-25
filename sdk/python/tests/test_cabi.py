import ctypes
import json
import unittest

from easynet_sdk import (
    ConnectOptions,
    ErrorCode,
    BidiFrame,
    BidiState,
    BidiStreamDescriptor,
    RuntimeLifecycle,
    RuntimeClient,
    SDKError,
    StreamState,
)
from easynet_sdk._cabi import (
    CABIRuntimeLifecycleTransport,
    CABIRuntimeTransport,
    CLILibrary,
    EXPECTED_ABI_VERSION,
    MAX_CABI_CALLBACK_QUEUE,
    _CABIStreamTransport,
    _platform_library_candidates,
    _project_cabi_ordered_event,
    _runtime_status_from_cabi,
    _runtime_start_config_for_cabi,
    _resolve_descriptor_ref_from_diagnostics,
)
from easynet_sdk.providers.runtime.lifecycle import (
    RuntimeHostMode,
    RuntimeHostStartConfig,
)

from test_runtime import canonical_runtime_receipt_pair, complete_draft


class CABIDescriptorDiagnosticsTests(unittest.TestCase):
    def test_descriptor_diagnostics_owner_mismatch_is_descriptor_not_found(self) -> None:
        diagnostics = {
            "descriptor_catalog": {
                "source": "test",
                "entries": [
                    {
                        "name": "page.fetch",
                        "owner_ura": "easynet:///r/test/agent/alice.pages",
                        "ability_ura": "easynet:///r/test/ability/alice.pages.page.fetch",
                        "descriptor_ref": "easynet:///r/test/ability/alice.pages.page.fetch@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
                        "call_mode": "rpc",
                    }
                ],
            }
        }

        with self.assertRaises(SDKError) as raised:
            _resolve_descriptor_ref_from_diagnostics(
                b'{"callee_ura":"easynet:///r/test/device/dev-a","ability":"page.fetch","call_mode":"rpc"}',
                diagnostics,
            )

        self.assertEqual(raised.exception.code, ErrorCode.DESCRIPTOR_NOT_FOUND)
        self.assertIn("descriptor_ref not found", raised.exception.message)

    def test_descriptor_diagnostics_requires_call_mode(self) -> None:
        diagnostics = {
            "descriptor_catalog": {
                "entries": [
                    {
                        "name": "page.fetch",
                        "owner_ura": "easynet:///r/test/device/dev-a",
                        "ability_ura": "easynet:///r/test/ability/device.dev-a.page.fetch",
                        "descriptor_ref": "easynet:///r/test/ability/device.dev-a.page.fetch@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke",
                        "call_mode": "rpc",
                    }
                ]
            }
        }

        with self.assertRaises(SDKError) as raised:
            _resolve_descriptor_ref_from_diagnostics(
                b'{"callee_ura":"easynet:///r/test/device/dev-a","ability":"page.fetch"}',
                diagnostics,
            )

        self.assertEqual(raised.exception.code, ErrorCode.INVALID_ARGUMENT)
        self.assertIn("call_mode is required", raised.exception.message)

    def test_descriptor_diagnostics_rejects_matching_row_without_descriptor_ref(self) -> None:
        diagnostics = {
            "descriptor_catalog": {
                "source": "test",
                "entries": [
                    {
                        "name": "page.fetch",
                        "owner_ura": "easynet:///r/test/device/dev-a",
                        "ability_ura": "easynet:///r/test/ability/device.dev-a.page.fetch",
                        "call_mode": "rpc",
                    }
                ],
            }
        }

        with self.assertRaises(SDKError) as raised:
            _resolve_descriptor_ref_from_diagnostics(
                b'{"callee_ura":"easynet:///r/test/device/dev-a","ability":"page.fetch","call_mode":"rpc"}',
                diagnostics,
            )

        self.assertEqual(raised.exception.code, ErrorCode.INVALID_ARGUMENT)
        self.assertIn("descriptor catalog row", raised.exception.message)
        self.assertIn("missing descriptor_ref", raised.exception.message)


class FakeSymbol:
    def __init__(self, func):
        self.func = func
        self.argtypes = None
        self.restype = None

    def __call__(self, *args):
        return self.func(*args)


class CABIEventProjectionTests(unittest.TestCase):
    def test_error_frame_projects_string_fields(self) -> None:
        projected = _project_cabi_ordered_event(
            b'{"code":"REMOTE_FAILED","message":"dispatch failed"}',
            lambda observed: observed or 7,
            use_observed_sequence=True,
        )

        self.assertEqual(
            json.loads(projected),
            {
                "code": "REMOTE_FAILED",
                "error": {
                    "code": "REMOTE_FAILED",
                    "message": "dispatch failed",
                },
                "message": "dispatch failed",
                "sequence": 7,
            },
        )

    def test_bidi_event_projection_keeps_canonical_data_frame(self) -> None:
        projected = _project_cabi_ordered_event(
            b'{"kind":"data","payload_base64":"aGVsbG8="}',
            lambda observed: observed or 1,
            use_observed_sequence=True,
        )

        self.assertEqual(
            json.loads(projected),
            {
                "kind": "data",
                "payload_base64": "aGVsbG8=",
                "sequence": 1,
            },
        )

    def test_error_frame_rejects_non_string_fields_without_crashing(self) -> None:
        projected = _project_cabi_ordered_event(
            b'{"code":500,"message":null}',
            lambda observed: observed or 1,
            use_observed_sequence=True,
        )

        self.assertEqual(json.loads(projected)["error"], {"code": "", "message": ""})

    def test_stream_allocator_normalizes_zero_based_remote_sequences(self) -> None:
        transport = _CABIStreamTransport(None, 1, 1, None)

        first = json.loads(
            _project_cabi_ordered_event(
                b'{"sequence":0,"kind":"data"}',
                transport._allocate_sequence,
                use_observed_sequence=True,
            )
        )
        second = json.loads(
            _project_cabi_ordered_event(
                b'{"sequence":1,"kind":"data"}',
                transport._allocate_sequence,
                use_observed_sequence=True,
            )
        )

        self.assertEqual(first["sequence"], 1)
        self.assertEqual(second["sequence"], 2)


class FakeRawCABI:
    """Strict generic C ABI fake: product-specific symbol lookups cannot succeed."""

    def __init__(self) -> None:
        self.buffers: dict[int, ctypes.Array[ctypes.c_char]] = {}
        self.callback_buffers: list[ctypes.Array[ctypes.c_char]] = []
        self.last_error_json = b"null"
        self.init_paths: list[str] = []
        self.shutdown_handles: list[int] = []
        self.runtime_requests: list[tuple[str, object]] = []
        self.prepared_frees: list[int] = []
        self.signed_frees: list[int] = []
        self.handle_frees: list[tuple[int, int]] = []
        self.stream_closes: list[int] = []
        self.stream_cancels: list[int] = []
        self.stream_callbacks: dict[int, tuple[object, object]] = {}
        self.bidi_sends: list[dict[str, object]] = []
        self.bidi_close_sends: list[int] = []
        self.bidi_closes: list[int] = []
        self.bidi_cancels: list[int] = []
        self.bidi_callbacks: dict[int, tuple[object, object]] = {}
        self.bidi_open_requests: list[dict[str, object]] = []
        self.overflow_callbacks = False
        self.runtime_host_starts: list[dict[str, object]] = []
        self.runtime_host_attaches: list[dict[str, object]] = []
        self.runtime_host_discovers: list[dict[str, object]] = []
        self.runtime_host_stops: list[int] = []
        self.runtime_host_detaches: list[int] = []
        self.runtime_host_open_clients: list[int] = []
        self.runtime_host_invocation_endpoint_calls: list[int] = []
        self.next_prepared_id = 1001

        self.runtime_abi_version = FakeSymbol(lambda: EXPECTED_ABI_VERSION)
        self.runtime_feature_discovery = FakeSymbol(self._feature_discovery)
        self.runtime_last_error_json = FakeSymbol(self._last_error_json)
        self.runtime_error_json = FakeSymbol(self._error_json)
        self.runtime_string_free = FakeSymbol(self._free)
        self.runtime_init = FakeSymbol(self._init)
        self.runtime_shutdown = FakeSymbol(self._shutdown)
        self.runtime_host_start = FakeSymbol(self._runtime_host_start)
        self.runtime_host_attach = FakeSymbol(self._runtime_host_attach)
        self.runtime_host_discover = FakeSymbol(self._runtime_host_discover)
        self.runtime_host_stop = FakeSymbol(self._runtime_host_stop)
        self.runtime_host_detach = FakeSymbol(self._runtime_host_detach)
        self.runtime_host_status = FakeSymbol(self._runtime_host_status)
        self.runtime_host_endpoints = FakeSymbol(self._runtime_host_endpoints)
        self.runtime_host_invocation_endpoint = FakeSymbol(
            self._runtime_host_invocation_endpoint
        )
        self.runtime_host_open_client = FakeSymbol(self._runtime_host_open_client)
        self.runtime_health = FakeSymbol(self._runtime_health)
        self.runtime_diagnostics = FakeSymbol(self._runtime_diagnostics)
        self.runtime_resolve_descriptor_ref = FakeSymbol(
            self._runtime_resolve_descriptor_ref
        )
        self.runtime_invocation_invoke = FakeSymbol(self._invocation_invoke)
        self.runtime_invocation_prepare = FakeSymbol(self._invocation_prepare)
        self.runtime_invocation_sign_prepared = FakeSymbol(
            self._invocation_sign_prepared
        )
        self.runtime_invocation_sign_prepared_local = FakeSymbol(
            self._invocation_sign_prepared_local
        )
        self.runtime_invocation_submit_signed_handle = FakeSymbol(
            self._invocation_submit_signed_handle
        )
        self.runtime_invocation_handle_await = FakeSymbol(self._invocation_handle_await)
        self.runtime_invocation_handle_cancel = FakeSymbol(
            self._invocation_handle_cancel
        )
        self.runtime_invocation_handle_events = FakeSymbol(
            self._invocation_handle_events
        )
        self.runtime_invocation_handle_free = FakeSymbol(self._invocation_handle_free)
        self.runtime_prepared_invocation_free = FakeSymbol(
            self._prepared_invocation_free
        )
        self.runtime_signed_invocation_free = FakeSymbol(self._signed_invocation_free)
        self.runtime_invocation_stream_open = FakeSymbol(self._invocation_stream_open)
        self.runtime_invocation_stream_cancel = FakeSymbol(
            self._invocation_stream_cancel
        )
        self.runtime_invocation_stream_close = FakeSymbol(self._invocation_stream_close)
        self.runtime_invocation_bidi_open = FakeSymbol(self._invocation_bidi_open)
        self.runtime_invocation_bidi_send = FakeSymbol(self._invocation_bidi_send)
        self.runtime_invocation_bidi_close_send = FakeSymbol(
            self._invocation_bidi_close_send
        )
        self.runtime_invocation_bidi_close = FakeSymbol(self._invocation_bidi_close)
        self.runtime_invocation_bidi_cancel = FakeSymbol(self._invocation_bidi_cancel)

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
            json.dumps(
                {
                    "abi_version": EXPECTED_ABI_VERSION,
                    "sdk_version": "0.91.30",
                    "profiles": {"runtime_core": "provider-backed"},
                    "symbols": {"generic_invocation": True},
                    "axon_pb": True,
                },
                separators=(",", ":"),
            ).encode("utf-8"),
        )

    def _last_error_json(self, out_ptr) -> int:
        return self._write(out_ptr, self.last_error_json)

    def _error_json(self, code, message, out_ptr) -> int:
        _ = message
        return self._write(
            out_ptr,
            json.dumps(
                {
                    "code": "GENERIC" if int(code) else "OK",
                    "stage": "cabi",
                    "message": "",
                    "retry": "never",
                    "details": {},
                },
                separators=(",", ":"),
            ).encode("utf-8"),
        )

    def _init(self, control_path, out_handle) -> int:
        if control_path is None:
            self.init_paths.append("")
        elif isinstance(control_path, bytes):
            self.init_paths.append(control_path.decode("utf-8"))
        else:
            self.init_paths.append(control_path.value.decode("utf-8"))
        out_handle._obj.value = 42
        return 0

    def _shutdown(self, handle) -> int:
        self.shutdown_handles.append(int(handle.value))
        return 0

    def _runtime_host_start(self, config_json, out_handle) -> int:
        self.runtime_host_starts.append(json.loads(config_json.value.decode("utf-8")))
        out_handle._obj.value = 606
        return 0

    def _runtime_host_attach(self, options_json, out_handle) -> int:
        self.runtime_host_attaches.append(json.loads(options_json.value.decode("utf-8")))
        out_handle._obj.value = 707
        return 0

    def _runtime_host_discover(self, options_json, out_ptr) -> int:
        self.runtime_host_discovers.append(json.loads(options_json.value.decode("utf-8")))
        return self._write(out_ptr, _RUNTIME_HOST_STATUS)

    def _runtime_host_stop(self, runtime_host_handle) -> int:
        self.runtime_host_stops.append(int(runtime_host_handle.value))
        return 0

    def _runtime_host_detach(self, runtime_host_handle) -> int:
        self.runtime_host_detaches.append(int(runtime_host_handle.value))
        return 0

    def _runtime_host_status(self, runtime_host_handle, out_ptr) -> int:
        _ = runtime_host_handle
        return self._write(out_ptr, _RUNTIME_HOST_STATUS)

    def _runtime_host_endpoints(self, runtime_host_handle, out_ptr) -> int:
        _ = runtime_host_handle
        return self._write(
            out_ptr,
            b'{"control_endpoint":"unix:///tmp/control.sock",'
            b'"invocation_endpoint":"unix:///tmp/runtime-host.sock",'
            b'"public_endpoint":null}',
        )

    def _runtime_host_invocation_endpoint(self, runtime_host_handle, out_ptr) -> int:
        self.runtime_host_invocation_endpoint_calls.append(int(runtime_host_handle.value))
        return self._write(out_ptr, b"unix:///tmp/runtime-host.sock")

    def _runtime_host_open_client(self, runtime_host_handle, out_handle) -> int:
        self.runtime_host_open_clients.append(int(runtime_host_handle.value))
        out_handle._obj.value = 808
        return 0

    def _runtime_health(self, handle, out_ptr) -> int:
        self.runtime_requests.append(("health", int(handle.value)))
        return self._write(
            out_ptr,
            b'{"api_ready":true,"invocation_ready":true,"directory_ready":true,'
            b'"trust_ready":true,"runtime_ready":true,"diagnostics":[]}',
        )

    def _runtime_diagnostics(self, handle, out_ptr) -> int:
        self.runtime_requests.append(("diagnostics", int(handle.value)))
        return self._write(
            out_ptr,
            json.dumps(
                {
                    "profile": "health",
                    "kind": "diagnostics_report",
                    "state": "Running",
                    "ready": True,
                    "abi_version": EXPECTED_ABI_VERSION,
                    "checks": [],
                    "diagnostics": [],
                },
                separators=(",", ":"),
            ).encode("utf-8"),
        )

    def _runtime_resolve_descriptor_ref(self, handle, request_json, out_ptr) -> int:
        _ = handle, request_json, out_ptr
        self.last_error_json = (
            b'{"code":"DESCRIPTOR_NOT_FOUND","stage":"routing","message":'
            b'"descriptor_ref not found","retry":"never","details":{}}'
        )
        return 1

    def _invocation_invoke(self, handle, raw, out_ptr) -> int:
        draft = json.loads(raw.value.decode("utf-8"))
        self.runtime_requests.append(("invoke", draft))
        admission, terminal = canonical_runtime_receipt_pair("inv-cabi")
        return self._write(
            out_ptr,
            json.dumps(
                {
                    "ok": True,
                    "tuple": draft,
                    "invocation_id": "inv-cabi",
                    "terminal_state": "Completed",
                    "output_json": {"ready": True},
                    "admission_receipt": admission,
                    "terminal_receipt": terminal,
                },
                separators=(",", ":"),
                sort_keys=True,
            ).encode("utf-8"),
        )

    def _invocation_prepare(
        self, handle, invocation_json, options_json, out_id, out_ptr
    ) -> int:
        _ = handle, invocation_json
        options = json.loads(options_json.value.decode("utf-8"))
        if options.get("material_only") is True:
            out_id._obj.value = 0
            return self._write(
                out_ptr,
                json.dumps(
                    {
                        "signing_material": {
                            "canonical_bytes_base64": "e30=",
                            "canonical_hash_hex": "00",
                        }
                    },
                    separators=(",", ":"),
                ).encode("utf-8"),
            )

        prepared_id = self.next_prepared_id
        self.next_prepared_id += 1
        out_id._obj.value = prepared_id
        return self._write(
            out_ptr,
            json.dumps(
                {
                    "prepared_id": str(prepared_id),
                    "request_id": "same-request-id",
                    "canonical_bytes_base64": "e30=",
                    "canonical_hash_hex": "00",
                },
                separators=(",", ":"),
            ).encode("utf-8"),
        )

    def _invocation_sign_prepared(
        self, prepared_id, signature_json, out_id, out_ptr
    ) -> int:
        _ = prepared_id, signature_json
        out_id._obj.value = 2001
        return self._write(out_ptr, b'{"signed_id":"signed-1"}')

    def _invocation_sign_prepared_local(self, prepared_id, out_id, out_ptr) -> int:
        _ = prepared_id
        out_id._obj.value = 2002
        return self._write(out_ptr, b'{"signed_id":"signed-local"}')

    def _invocation_submit_signed_handle(
        self, handle, signed_id, out_handle, out_ptr
    ) -> int:
        _ = handle, signed_id
        out_handle._obj.value = 3001
        return self._write(out_ptr, b'{"handle_id":3001,"state":"Submitted"}')

    def _invocation_handle_await(self, handle, invocation_handle_id, out_ptr) -> int:
        _ = handle, invocation_handle_id
        return self._write(
            out_ptr,
            b'{"ok":true,"terminal_state":"Completed","output_json":{"ready":true}}',
        )

    def _invocation_handle_cancel(
        self, handle, invocation_handle_id, reason, out_ptr
    ) -> int:
        _ = handle, reason
        return self._write(
            out_ptr,
            json.dumps(
                {"handle_id": int(invocation_handle_id.value), "cancelled": True}
            ).encode("utf-8"),
        )

    def _invocation_handle_events(self, handle, invocation_handle_id, out_ptr) -> int:
        _ = handle, invocation_handle_id
        return self._write(out_ptr, b'{"events":[]}')

    def _invocation_handle_free(self, handle, invocation_handle_id) -> int:
        self.handle_frees.append((int(handle.value), int(invocation_handle_id.value)))
        return 0

    def _prepared_invocation_free(self, prepared_id) -> int:
        self.prepared_frees.append(int(prepared_id.value))
        return 0

    def _signed_invocation_free(self, signed_id) -> int:
        self.signed_frees.append(int(signed_id.value))
        return 0

    def _callback(self, callback, user_data, payload: bytes) -> None:
        buffer = ctypes.create_string_buffer(payload)
        self.callback_buffers.append(buffer)
        callback(user_data, ctypes.c_void_p(ctypes.addressof(buffer)))

    def _invocation_stream_open(
        self, handle, invocation_json, callback, user_data, out_stream_id
    ) -> int:
        _ = handle, invocation_json
        out_stream_id._obj.value = 4001
        self.stream_callbacks[4001] = (callback, user_data)
        if self.overflow_callbacks:
            for sequence in range(1, MAX_CABI_CALLBACK_QUEUE + 2):
                self._callback(
                    callback,
                    user_data,
                    json.dumps(
                        {
                            "sequence": sequence,
                            "kind": "data",
                            "state": "Open",
                            "terminal": False,
                        },
                        separators=(",", ":"),
                    ).encode("utf-8"),
                )
        else:
            self._callback(
                callback,
                user_data,
                b'{"sequence":1,"kind":"data","state":"Open",'
                b'"terminal":false,"payload_json":{"provider":"cabi"}}',
            )
        return 0

    def _invocation_stream_cancel(self, handle, stream_id) -> int:
        _ = handle
        native_id = int(stream_id.value)
        self.stream_cancels.append(native_id)
        callback_state = self.stream_callbacks.get(native_id)
        if callback_state is not None:
            self._callback(
                *callback_state,
                b'{"sequence":2,"kind":"terminal","state":"Cancelled",'
                b'"terminal":true,"terminal_receipt":'
                b'{"state":"Cancelled","cleanup_complete":true}}',
            )
        return 0

    def _invocation_stream_close(self, handle, stream_id) -> int:
        _ = handle
        native_id = int(stream_id.value)
        self.stream_closes.append(native_id)
        self.stream_callbacks.pop(native_id, None)
        return 0

    def _invocation_bidi_open(
        self, handle, invocation_json, callback, user_data, out_bidi_id
    ) -> int:
        _ = handle
        request = json.loads(invocation_json.value.decode("utf-8"))
        self.bidi_open_requests.append(request)
        if not request.get("bidi_streams"):
            self.last_error_json = (
                b'{"code":"INVALID_ARGUMENT","stage":"cabi","message":'
                b'"bidi frame0 is required","retry":"never","details":{}}'
            )
            return 1
        out_bidi_id._obj.value = 5001
        self.bidi_callbacks[5001] = (callback, user_data)
        if self.overflow_callbacks:
            for sequence in range(1, MAX_CABI_CALLBACK_QUEUE + 2):
                self._callback(
                    callback,
                    user_data,
                    json.dumps(
                        {
                            "sequence": sequence,
                            "kind": "data",
                            "stream_id": 1,
                            "terminal": False,
                        },
                        separators=(",", ":"),
                    ).encode("utf-8"),
                )
        else:
            self._callback(
                callback,
                user_data,
                b'{"sequence":1,"kind":"data","stream_id":1,'
                b'"terminal":false,"payload_json":{"provider":"cabi"}}',
            )
        return 0

    def _invocation_bidi_send(self, handle, bidi_id, frame_json) -> int:
        _ = handle, bidi_id
        self.bidi_sends.append(json.loads(frame_json.value.decode("utf-8")))
        return 0

    def _invocation_bidi_close_send(self, handle, bidi_id) -> int:
        _ = handle
        self.bidi_close_sends.append(int(bidi_id.value))
        return 0

    def _invocation_bidi_close(self, handle, bidi_id) -> int:
        _ = handle
        native_id = int(bidi_id.value)
        self.bidi_closes.append(native_id)
        self.bidi_callbacks.pop(native_id, None)
        return 0

    def _invocation_bidi_cancel(self, handle, bidi_id) -> int:
        _ = handle
        native_id = int(bidi_id.value)
        self.bidi_cancels.append(native_id)
        callback_state = self.bidi_callbacks.get(native_id)
        if callback_state is not None:
            self._callback(
                *callback_state,
                b'{"sequence":2,"kind":"terminal","stream_id":1,'
                b'"terminal":true,"terminal_receipt":'
                b'{"state":"Cancelled","cleanup_complete":true}}',
            )
        return 0


_RUNTIME_HOST_STATUS = (
    b'{"state":"Running","mode":"edge","pid":123,'
    b'"version":"0.91.30","message":"ready","diagnostics":[],'
    b'"control_endpoint":"unix:///tmp/control.sock",'
    b'"invocation_endpoint":"unix:///tmp/runtime-host.sock",'
    b'"invocation_accepting":true}'
)


class CABITransportTests(unittest.TestCase):
    def test_default_library_candidates_never_probe_repository_targets(self) -> None:
        candidates = _platform_library_candidates()

        self.assertEqual(len(candidates), 1)
        self.assertFalse(any("target/" in candidate for candidate in candidates))

    def test_library_binds_only_generic_v6_symbols(self) -> None:
        raw = FakeRawCABI()
        library = CLILibrary(raw)

        library.require_abi()
        features = json.loads(library.feature_discovery())

        self.assertEqual(EXPECTED_ABI_VERSION, 6)
        self.assertEqual(features["abi_version"], 6)
        self.assertEqual(features["profiles"], {"runtime_core": "provider-backed"})

    def test_library_exposes_runtime_host_not_daemon_lifecycle_methods(self) -> None:
        lifecycle_methods = {
            name
            for name in dir(CLILibrary)
            if name.endswith(
                (
                    "_start",
                    "_attach",
                    "_discover",
                    "_stop",
                    "_detach",
                    "_status",
                    "_endpoints",
                    "_invocation_endpoint",
                    "_open_client",
                )
            )
        }

        self.assertIn("runtime_host_start", lifecycle_methods)
        self.assertIn("runtime_host_open_client", lifecycle_methods)
        self.assertFalse(
            [name for name in lifecycle_methods if name.startswith("daemon_")]
        )

    def test_runtime_host_uses_generic_invocation(self) -> None:
        raw = FakeRawCABI()
        lifecycle = CABIRuntimeLifecycleTransport(CLILibrary(raw))
        self.assertEqual(
            RuntimeHostStartConfig(
                mode=RuntimeHostMode.EDGE,
                runtime_instance_id="dev-a",
            ).to_json_dict()["mode"],
            "edge",
        )
        handle = RuntimeLifecycle(lifecycle).start(
            RuntimeHostStartConfig(
                mode=RuntimeHostMode.EDGE,
                runtime_instance_id="dev-a",
            )
        )
        self.assertEqual(handle.status().mode, "edge")
        runtime = handle.open_runtime(ConnectOptions())

        result = runtime.invoke(complete_draft())

        self.assertTrue(result.ok)
        self.assertEqual(result.output_json, {"ready": True})
        self.assertEqual(raw.runtime_host_open_clients, [606])
        self.assertEqual(raw.runtime_host_starts[0]["mode"], "edge")

    def test_runtime_host_start_rejects_retired_product_mode_input(self) -> None:
        for mode in ("device", "hub", "both"):
            with self.subTest(mode=mode):
                with self.assertRaises(SDKError) as caught:
                    _runtime_start_config_for_cabi(
                        json.dumps({"mode": mode}).encode("utf-8")
                    )

                self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)
                self.assertIn("edge, authority, or combined", caught.exception.message)

    def test_runtime_host_start_rejects_unsupported_combined_mode(self) -> None:
        with self.assertRaises(SDKError) as caught:
            _runtime_start_config_for_cabi(b'{"mode":"combined"}')

        self.assertEqual(caught.exception.code, ErrorCode.NOT_IMPLEMENTED)
        self.assertIn(
            "does not support combined runtime host mode", caught.exception.message
        )

    def test_runtime_host_status_rejects_retired_product_modes(self) -> None:
        for mode in ("device", "hub", "both"):
            with self.subTest(mode=mode):
                with self.assertRaises(SDKError) as caught:
                    _runtime_status_from_cabi(
                        "42",
                        json.dumps(
                            {
                                "state": "Running",
                                "mode": mode,
                                "endpoints": {
                                    "control_endpoint": "unix:///tmp/control.sock"
                                },
                            }
                        ).encode("utf-8"),
                    )

                self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)
                self.assertIn("edge, authority, or combined", caught.exception.message)

    def test_runtime_host_status_accepts_canonical_combined_mode(self) -> None:
        status = _runtime_status_from_cabi(
            "42",
            b'{"state":"Running","mode":"combined","endpoints":'
            b'{"control_endpoint":"unix:///tmp/control.sock"}}',
        )

        self.assertEqual(status["mode"], "combined")

    def test_runtime_host_start_rejects_unknown_mode_input(self) -> None:
        with self.assertRaises(SDKError) as caught:
            _runtime_start_config_for_cabi(b'{"mode":"daemon"}')

        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)
        self.assertIn("edge, authority, or combined", caught.exception.message)

    def test_runtime_handle_has_no_product_profile_factory(self) -> None:
        raw = FakeRawCABI()
        lifecycle = CABIRuntimeLifecycleTransport(CLILibrary(raw))
        handle = RuntimeLifecycle(lifecycle).start(
            RuntimeHostStartConfig(
                mode=RuntimeHostMode.EDGE,
                runtime_instance_id="dev-a",
            )
        )

        self.assertFalse(hasattr(handle, "identity"))

    def test_runtime_transport_closes_owned_handle_once(self) -> None:
        raw = FakeRawCABI()
        transport = CABIRuntimeTransport(CLILibrary(raw), 42, owns_handle=True)
        runtime = RuntimeClient(transport)

        runtime.close()
        runtime.close()

        self.assertEqual(raw.shutdown_handles, [42])

    def test_descriptor_resolution_uses_native_runtime_provider(
        self,
    ) -> None:
        caller_ura = "easynet:///r/acme/device/caller"
        callee_ura = "easynet:///r/acme/device/provider"
        target_ability = "easynet:///r/acme/ability/device.provider.er.add"
        target_ref = f"{target_ability}@1.0.0#{'b' * 64}!stream"

        class NativeDescriptorRaw(FakeRawCABI):
            def _runtime_resolve_descriptor_ref(
                self, handle, request_json, out_ptr
            ) -> int:
                request = json.loads(request_json.value.decode("utf-8"))
                self.runtime_requests.append(("resolve_descriptor_ref", request))
                return self._write(
                    out_ptr,
                    json.dumps(
                        {
                            "descriptor_ref": target_ref,
                            "ability_ura": target_ability,
                            "owner_ura": callee_ura,
                            "name": "er.add",
                            "call_mode": "stream",
                            "source": "fake_native_provider",
                        },
                        separators=(",", ":"),
                        sort_keys=True,
                    ).encode("utf-8"),
                )

        raw = NativeDescriptorRaw()
        transport = CABIRuntimeTransport(CLILibrary(raw), 42, owns_handle=False)

        resolved = json.loads(
            transport.resolve_descriptor_ref(
                json.dumps(
                    {
                        "callee_ura": callee_ura,
                        "ability": target_ability,
                        "call_mode": "stream",
                        "caller_ura": caller_ura,
                    },
                    separators=(",", ":"),
                    sort_keys=True,
                ).encode("utf-8")
            ).decode("utf-8")
        )

        self.assertEqual(resolved["descriptor_ref"], target_ref)
        resolutions = [
            request
            for kind, request in raw.runtime_requests
            if kind == "resolve_descriptor_ref"
        ]
        self.assertEqual(len(resolutions), 1)
        self.assertEqual(resolutions[0]["callee_ura"], callee_ura)
        self.assertEqual(resolutions[0]["ability"], target_ability)
        self.assertFalse(
            any(kind == "diagnostics" or kind == "invoke" for kind, _ in raw.runtime_requests)
        )

    def test_descriptor_resolution_projects_native_last_error(self) -> None:
        transport = CABIRuntimeTransport(CLILibrary(FakeRawCABI()), 42, owns_handle=False)

        with self.assertRaises(SDKError) as raised:
            transport.resolve_descriptor_ref(
                json.dumps(
                    {
                        "callee_ura": "easynet:///r/acme/device/provider",
                        "ability": "missing.descriptor",
                        "call_mode": "rpc",
                    },
                    separators=(",", ":"),
                    sort_keys=True,
                ).encode("utf-8")
            )

        self.assertEqual(raised.exception.code, ErrorCode.DESCRIPTOR_NOT_FOUND)
        self.assertEqual(raised.exception.stage, "routing")
        self.assertNotIn("C ABI", raised.exception.message)

    def test_prepare_uses_opaque_c_handle_when_request_id_repeats(self) -> None:
        raw = FakeRawCABI()
        transport = CABIRuntimeTransport(CLILibrary(raw), 42, owns_handle=False)

        first = json.loads(transport.prepare(b"{}", b"{}").decode("utf-8"))
        second = json.loads(transport.prepare(b"{}", b"{}").decode("utf-8"))

        self.assertEqual(first["request_id"], second["request_id"])
        self.assertNotEqual(first["prepared_id"], second["prepared_id"])
        self.assertEqual(transport._prepared_handles.keys(), {"1001", "1002"})

    def test_prepare_rejects_duplicate_prepared_handle_id(self) -> None:
        raw = FakeRawCABI()
        transport = CABIRuntimeTransport(CLILibrary(raw), 42, owns_handle=False)

        first = json.loads(transport.prepare(b"{}", b"{}").decode("utf-8"))
        raw.next_prepared_id = int(first["prepared_id"])

        with self.assertRaises(SDKError) as caught:
            transport.prepare(b"{}", b"{}")

        self.assertEqual(caught.exception.code, ErrorCode.PROTOCOL)
        self.assertIn("duplicate prepared handle id", caught.exception.message)
        self.assertEqual(raw.prepared_frees, [int(first["prepared_id"])])
        self.assertEqual(transport._prepared_handles.keys(), {first["prepared_id"]})

    def test_prepare_material_only_does_not_retain_prepared_handle(self) -> None:
        raw = FakeRawCABI()
        transport = CABIRuntimeTransport(CLILibrary(raw), 42, owns_handle=False)

        material = json.loads(
            transport.prepare(b"{}", b'{"material_only":true}').decode("utf-8")
        )

        self.assertEqual(
            material["signing_material"]["canonical_bytes_base64"],
            "e30=",
        )
        self.assertEqual(transport._prepared_handles.keys(), set())
        self.assertEqual(raw.prepared_frees, [])

    def test_submit_signed_rejects_request_id_only_prepared_reference(self) -> None:
        raw = FakeRawCABI()
        transport = CABIRuntimeTransport(CLILibrary(raw), 42, owns_handle=False)
        transport.prepare(b"{}", b"{}")

        with self.assertRaises(SDKError) as caught:
            transport.submit_signed(
                b'{"prepared":{"request_id":"same-request-id"},'
                b'"signature":{"algorithm":"ed25519","signature_base64":"abc"}}'
            )

        self.assertEqual(caught.exception.code, ErrorCode.INVALID_ARGUMENT)

    def test_cabi_provider_requests_stream_cancel_before_canonical_terminal(
        self,
    ) -> None:
        raw, stream_provider, stream_cancel, stream_terminal, stream = (
            self._observe_stream_lifecycle()
        )

        self.assertFalse(stream_provider.terminal)
        self.assertEqual(stream_cancel.state, StreamState.CANCEL_REQUESTED)
        self.assertFalse(stream_cancel.cancelled)
        self.assertFalse(stream_cancel.terminal)
        self.assertTrue(stream_terminal.terminal)
        self.assertIsNotNone(stream_terminal.terminal_receipt)
        self.assertEqual(raw.stream_cancels, [4001])
        self.assertEqual(stream.runtime_state, StreamState.TERMINAL_FRAME_SEEN)

    def test_cabi_provider_dispatches_stream_before_terminal(self) -> None:
        _, stream_provider, _, stream_terminal, _ = self._observe_stream_lifecycle()

        self.assertFalse(stream_provider.terminal)
        self.assertEqual(stream_provider.payload_json, {"provider": "cabi"})
        self.assertLess(stream_provider.sequence, stream_terminal.sequence)
        self.assertIsNotNone(stream_terminal.terminal_receipt)

    def test_cabi_provider_preserves_stream_order_and_single_terminal(self) -> None:
        _, stream_provider, _, stream_terminal, stream = (
            self._observe_stream_lifecycle()
        )

        self.assertEqual(stream_provider.sequence, 1)
        self.assertEqual(stream_terminal.sequence, 2)
        self.assertEqual(len(stream.events), 2)
        self.assertEqual(sum(event.terminal for event in stream.events), 1)
        self.assertEqual(stream.state, StreamState.CLOSED)
        self.assertEqual(stream.runtime_state, StreamState.TERMINAL_FRAME_SEEN)

    def test_cabi_provider_requests_bidi_cancel_before_canonical_terminal(
        self,
    ) -> None:
        raw, bidi_provider, bidi_cancel, bidi_terminal, session = (
            self._observe_bidi_lifecycle()
        )

        self.assertFalse(bidi_provider.terminal)
        self.assertEqual(bidi_cancel.state, BidiState.CANCEL_REQUESTED)
        self.assertFalse(bidi_cancel.terminal)
        self.assertTrue(bidi_terminal.terminal)
        self.assertIsNotNone(bidi_terminal.terminal_receipt)
        self.assertEqual(raw.bidi_cancels, [5001])
        self.assertEqual(session.runtime_state, BidiState.TERMINAL)

    def test_cabi_provider_dispatches_bidi_before_terminal(self) -> None:
        _, bidi_provider, _, bidi_terminal, session = self._observe_bidi_lifecycle()

        self.assertFalse(bidi_provider.terminal)
        self.assertEqual(bidi_provider.payload_json, {"provider": "cabi"})
        self.assertIsNotNone(bidi_terminal.terminal_receipt)
        self.assertEqual(session.state, BidiState.CLOSED)
        self.assertEqual(session.runtime_state, BidiState.TERMINAL)

    def _observe_stream_lifecycle(self):
        raw = FakeRawCABI()
        runtime = RuntimeClient(
            CABIRuntimeTransport(CLILibrary(raw), 42, owns_handle=False)
        )
        self.addCleanup(runtime.close)

        stream = runtime.invoke_stream(complete_draft())
        stream_provider = stream.next()
        stream_cancel = stream.cancel("client stop")
        stream_terminal = stream.next()
        stream.close()
        return raw, stream_provider, stream_cancel, stream_terminal, stream

    def _observe_bidi_lifecycle(self):
        raw = FakeRawCABI()
        runtime = RuntimeClient(
            CABIRuntimeTransport(CLILibrary(raw), 42, owns_handle=False)
        )
        self.addCleanup(runtime.close)
        session = runtime.open_bidi(
            complete_draft(),
            (
                BidiStreamDescriptor(
                    stream_id=1, content_type="application/json", ordering="STRICT"
                ),
            ),
        )
        bidi_provider = session.receive()
        bidi_cancel = session.cancel("client stop")
        bidi_terminal = session.receive()
        session.close()
        return raw, bidi_provider, bidi_cancel, bidi_terminal, session

    def test_cabi_provider_enforces_callback_backpressure(self) -> None:
        raw = FakeRawCABI()
        raw.overflow_callbacks = True
        runtime = RuntimeClient(
            CABIRuntimeTransport(CLILibrary(raw), 42, owns_handle=False)
        )
        self.addCleanup(runtime.close)

        stream = runtime.invoke_stream(complete_draft())
        stream_failure = stream.next()

        self._assert_backpressure_failure(
            stream_failure.terminal,
            stream_failure.transport_terminal,
            stream_failure.error,
        )
        self.assertEqual(stream.state, StreamState.FAILED)
        self.assertEqual(stream.runtime_state, StreamState.OPEN)
        stream.close()

        session = runtime.open_bidi(
            complete_draft(),
            (
                BidiStreamDescriptor(
                    stream_id=1, content_type="application/json", ordering="STRICT"
                ),
            ),
        )
        bidi_failure = session.receive()

        self._assert_backpressure_failure(
            bidi_failure.terminal,
            bidi_failure.transport_terminal,
            bidi_failure.error,
        )
        self.assertEqual(session.state, BidiState.FAILED)
        self.assertEqual(session.runtime_state, BidiState.OPEN)
        session.close()

    def test_cabi_provider_owns_stream_receive_deadline(self) -> None:
        raw = FakeRawCABI()
        runtime = RuntimeClient(
            CABIRuntimeTransport(CLILibrary(raw), 42, owns_handle=False)
        )
        self.addCleanup(runtime.close)

        stream = runtime.invoke_stream(complete_draft())
        stream.next()
        with self.assertRaises(SDKError) as stream_timeout:
            stream.next(timeout=0.01)
        self.assertEqual(stream_timeout.exception.code, ErrorCode.TIMEOUT)
        self.assertEqual(stream.runtime_state, StreamState.OPEN)
        stream.close()
        retry_stream = runtime.invoke_stream(complete_draft())
        self.assertFalse(retry_stream.next().terminal)
        retry_stream.close()

    def test_cabi_provider_owns_bidi_receive_deadline(self) -> None:
        raw = FakeRawCABI()
        runtime = RuntimeClient(
            CABIRuntimeTransport(CLILibrary(raw), 42, owns_handle=False)
        )
        self.addCleanup(runtime.close)
        session = runtime.open_bidi(
            complete_draft(),
            (
                BidiStreamDescriptor(
                    stream_id=1, content_type="application/json", ordering="STRICT"
                ),
            ),
        )
        session.receive()
        with self.assertRaises(SDKError) as bidi_timeout:
            session.receive(timeout=0.01)
        self.assertEqual(bidi_timeout.exception.code, ErrorCode.TIMEOUT)
        self.assertEqual(session.runtime_state, BidiState.OPEN)
        session.close()
        retry_bidi = runtime.open_bidi(
            complete_draft(),
            (
                BidiStreamDescriptor(
                    stream_id=1, content_type="application/json", ordering="STRICT"
                ),
            ),
        )
        self.assertFalse(retry_bidi.receive().terminal)
        retry_bidi.close()

    def test_cabi_provider_keeps_close_send_distinct_from_cancel(self) -> None:
        raw = FakeRawCABI()
        runtime = RuntimeClient(
            CABIRuntimeTransport(CLILibrary(raw), 42, owns_handle=False)
        )
        self.addCleanup(runtime.close)
        session = runtime.open_bidi(
            complete_draft(),
            (
                BidiStreamDescriptor(
                    stream_id=1, content_type="application/json", ordering="STRICT"
                ),
            ),
        )

        outcome = session.close_send()

        self.assertFalse(outcome.terminal)
        self.assertEqual(outcome.state, BidiState.HALF_CLOSED_LOCAL)
        self.assertEqual(session.runtime_state, BidiState.HALF_CLOSED_LOCAL)
        self.assertEqual(raw.bidi_close_sends, [5001])
        self.assertEqual(raw.bidi_cancels, [])
        self.assertFalse(session.receive().terminal)
        with self.assertRaises(SDKError) as send_after_close:
            session.send(BidiFrame(sequence=1, kind="data", stream_id=1))
        self.assertEqual(send_after_close.exception.code, ErrorCode.CANCELLED)
        with self.assertRaises(SDKError) as receive_timeout:
            session.receive(timeout=0.01)
        self.assertEqual(receive_timeout.exception.code, ErrorCode.TIMEOUT)
        self.assertEqual(raw.bidi_cancels, [])
        session.close()
        self.assertEqual(session.runtime_state, BidiState.HALF_CLOSED_LOCAL)

    def test_cabi_provider_rejects_missing_bidi_frame_zero(self) -> None:
        raw = FakeRawCABI()
        runtime = RuntimeClient(
            CABIRuntimeTransport(CLILibrary(raw), 42, owns_handle=False)
        )
        self.addCleanup(runtime.close)

        with self.assertRaises(SDKError) as raised:
            runtime.open_bidi(complete_draft(), ())

        self.assertEqual(raised.exception.code, ErrorCode.INVALID_ARGUMENT)
        self.assertEqual(raw.bidi_open_requests, [])

    def _assert_backpressure_failure(
        self, terminal: bool, transport_terminal: bool, error: object
    ) -> None:
        self.assertFalse(terminal)
        self.assertTrue(transport_terminal)
        self.assertIsInstance(error, dict)
        assert isinstance(error, dict)
        self.assertEqual(error["code"], ErrorCode.ADMISSION_DENIED)
        self.assertEqual(error["retry"], "after_backoff")
        details = error["details"]
        self.assertIsInstance(details, dict)
        assert isinstance(details, dict)
        self.assertEqual(details["wire_code"], "RESOURCE_EXHAUSTED")
        self.assertEqual(details["reason"], "callback_queue_overflow")
        self.assertTrue(details["bounded_queue"])


if __name__ == "__main__":
    unittest.main()
