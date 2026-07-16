import ctypes
import json
import unittest

from easynet_sdk import (
    ConnectOptions,
    DaemonControl,
    ErrorCode,
    BidiState,
    BidiStreamDescriptor,
    RuntimeHostRole,
    RuntimeLifecycle,
    RuntimeClient,
    SDKError,
    StartConfig,
    DaemonMode,
    StreamState,
)
from easynet_sdk._cabi import (
    CABIDaemonTransport,
    CABIRuntimeLifecycleTransport,
    CABIRuntimeTransport,
    CLILibrary,
    EXPECTED_ABI_VERSION,
    _platform_library_candidates,
    _project_cabi_ordered_event,
)

from test_runtime import complete_draft


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


class FakeRawCABI:
    """Strict generic-v5 fake: product-specific symbol lookups cannot succeed."""

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
        self.bidi_sends: list[dict[str, object]] = []
        self.bidi_close_sends: list[int] = []
        self.bidi_closes: list[int] = []
        self.bidi_cancels: list[int] = []
        self.daemon_starts: list[dict[str, object]] = []
        self.daemon_attaches: list[dict[str, object]] = []
        self.daemon_discovers: list[dict[str, object]] = []
        self.daemon_stops: list[int] = []
        self.daemon_detaches: list[int] = []
        self.daemon_open_clients: list[int] = []
        self.daemon_invocation_endpoint_calls: list[int] = []
        self.next_prepared_id = 1001

        self.easynet_abi_version = FakeSymbol(lambda: EXPECTED_ABI_VERSION)
        self.easynet_feature_discovery = FakeSymbol(self._feature_discovery)
        self.easynet_last_error_json = FakeSymbol(self._last_error_json)
        self.easynet_error_json = FakeSymbol(self._error_json)
        self.easynet_string_free = FakeSymbol(self._free)
        self.easynet_init = FakeSymbol(self._init)
        self.easynet_shutdown = FakeSymbol(self._shutdown)
        self.easynet_daemon_start = FakeSymbol(self._daemon_start)
        self.easynet_daemon_attach = FakeSymbol(self._daemon_attach)
        self.easynet_daemon_discover = FakeSymbol(self._daemon_discover)
        self.easynet_daemon_stop = FakeSymbol(self._daemon_stop)
        self.easynet_daemon_detach = FakeSymbol(self._daemon_detach)
        self.easynet_daemon_status = FakeSymbol(self._daemon_status)
        self.easynet_daemon_endpoints = FakeSymbol(self._daemon_endpoints)
        self.easynet_daemon_invocation_endpoint = FakeSymbol(
            self._daemon_invocation_endpoint
        )
        self.easynet_daemon_open_client = FakeSymbol(self._daemon_open_client)
        self.easynet_runtime_health = FakeSymbol(self._runtime_health)
        self.easynet_runtime_diagnostics = FakeSymbol(self._runtime_diagnostics)
        self.easynet_invocation_invoke = FakeSymbol(self._invocation_invoke)
        self.easynet_invocation_prepare = FakeSymbol(self._invocation_prepare)
        self.easynet_invocation_sign_prepared = FakeSymbol(
            self._invocation_sign_prepared
        )
        self.easynet_invocation_sign_prepared_local = FakeSymbol(
            self._invocation_sign_prepared_local
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
        self.easynet_signed_invocation_free = FakeSymbol(
            self._signed_invocation_free
        )
        self.easynet_invocation_stream_open = FakeSymbol(
            self._invocation_stream_open
        )
        self.easynet_invocation_stream_cancel = FakeSymbol(
            self._invocation_stream_cancel
        )
        self.easynet_invocation_stream_close = FakeSymbol(
            self._invocation_stream_close
        )
        self.easynet_invocation_bidi_open = FakeSymbol(self._invocation_bidi_open)
        self.easynet_invocation_bidi_send = FakeSymbol(self._invocation_bidi_send)
        self.easynet_invocation_bidi_close_send = FakeSymbol(
            self._invocation_bidi_close_send
        )
        self.easynet_invocation_bidi_close = FakeSymbol(self._invocation_bidi_close)
        self.easynet_invocation_bidi_cancel = FakeSymbol(self._invocation_bidi_cancel)

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
            b'{"abi_version":5,"sdk_version":"0.91.30",'
            b'"profiles":{"runtime_core":"provider-backed"},'
            b'"symbols":{"generic_invocation":true},"axon_pb":true}',
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

    def _daemon_start(self, config_json, out_handle) -> int:
        self.daemon_starts.append(json.loads(config_json.value.decode("utf-8")))
        out_handle._obj.value = 606
        return 0

    def _daemon_attach(self, options_json, out_handle) -> int:
        self.daemon_attaches.append(json.loads(options_json.value.decode("utf-8")))
        out_handle._obj.value = 707
        return 0

    def _daemon_discover(self, options_json, out_ptr) -> int:
        self.daemon_discovers.append(json.loads(options_json.value.decode("utf-8")))
        return self._write(out_ptr, _DAEMON_STATUS)

    def _daemon_stop(self, daemon_handle) -> int:
        self.daemon_stops.append(int(daemon_handle.value))
        return 0

    def _daemon_detach(self, daemon_handle) -> int:
        self.daemon_detaches.append(int(daemon_handle.value))
        return 0

    def _daemon_status(self, daemon_handle, out_ptr) -> int:
        _ = daemon_handle
        return self._write(out_ptr, _DAEMON_STATUS)

    def _daemon_endpoints(self, daemon_handle, out_ptr) -> int:
        _ = daemon_handle
        return self._write(
            out_ptr,
            b'{"control_endpoint":"unix:///tmp/control.sock",'
            b'"invocation_endpoint":"unix:///tmp/daemon.sock",'
            b'"public_endpoint":null}',
        )

    def _daemon_invocation_endpoint(self, daemon_handle, out_ptr) -> int:
        self.daemon_invocation_endpoint_calls.append(int(daemon_handle.value))
        return self._write(out_ptr, b"unix:///tmp/daemon.sock")

    def _daemon_open_client(self, daemon_handle, out_handle) -> int:
        self.daemon_open_clients.append(int(daemon_handle.value))
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
            b'{"profile":"health","kind":"diagnostics_report",'
            b'"state":"Running","ready":true,"abi_version":5,'
            b'"checks":[],"diagnostics":[]}',
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
                    "output_json": {"ready": True},
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
            b'{"ok":true,"terminal_state":"Completed",'
            b'"output_json":{"ready":true}}',
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
        self._callback(
            callback,
            user_data,
            b'{"sequence":1,"kind":"terminal","terminal":true}',
        )
        return 0

    def _invocation_stream_cancel(self, handle, stream_id) -> int:
        _ = handle
        self.stream_cancels.append(int(stream_id.value))
        return 0

    def _invocation_stream_close(self, handle, stream_id) -> int:
        _ = handle
        self.stream_closes.append(int(stream_id.value))
        return 0

    def _invocation_bidi_open(
        self, handle, invocation_json, callback, user_data, out_bidi_id
    ) -> int:
        _ = handle, invocation_json
        out_bidi_id._obj.value = 5001
        self._callback(
            callback,
            user_data,
            b'{"sequence":1,"kind":"terminal","stream_id":1,"terminal":true}',
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
        self.bidi_closes.append(int(bidi_id.value))
        return 0

    def _invocation_bidi_cancel(self, handle, bidi_id) -> int:
        _ = handle
        self.bidi_cancels.append(int(bidi_id.value))
        return 0


_DAEMON_STATUS = (
    b'{"state":"Running","mode":"device","pid":123,'
    b'"version":"0.91.30","message":"ready","diagnostics":[],'
    b'"control_endpoint":"unix:///tmp/control.sock",'
    b'"invocation_endpoint":"unix:///tmp/daemon.sock",'
    b'"invocation_accepting":true}'
)


class CABITransportTests(unittest.TestCase):
    def test_default_library_candidates_never_probe_repository_targets(self) -> None:
        candidates = _platform_library_candidates()

        self.assertEqual(len(candidates), 1)
        self.assertFalse(any("target/" in candidate for candidate in candidates))

    def test_library_binds_only_generic_v5_symbols(self) -> None:
        raw = FakeRawCABI()
        library = CLILibrary(raw)

        library.require_abi()
        features = json.loads(library.feature_discovery())

        self.assertEqual(EXPECTED_ABI_VERSION, 5)
        self.assertEqual(features["abi_version"], 5)
        self.assertEqual(features["profiles"], {"runtime_core": "provider-backed"})

    def test_daemon_runtime_uses_generic_invocation(self) -> None:
        raw = FakeRawCABI()
        lifecycle = CABIRuntimeLifecycleTransport(CLILibrary(raw))
        handle = RuntimeLifecycle(lifecycle).start(
            StartConfig(mode=RuntimeHostRole.DEVICE, device_id="dev-a")
        )
        runtime = handle.open_runtime(ConnectOptions())

        result = runtime.invoke(complete_draft())

        self.assertTrue(result.ok)
        self.assertEqual(result.output_json, {"ready": True})
        self.assertEqual(raw.daemon_open_clients, [606])

    def test_daemon_handle_has_no_product_profile_factory(self) -> None:
        raw = FakeRawCABI()
        daemon = CABIDaemonTransport(CLILibrary(raw))
        handle = DaemonControl(daemon).start(
            StartConfig(mode=DaemonMode.DEVICE, device_id="dev-a")
        )

        self.assertFalse(hasattr(handle, "identity"))

    def test_daemon_transport_name_is_source_compatible_alias(self) -> None:
        self.assertIs(CABIDaemonTransport, CABIRuntimeLifecycleTransport)
        self.assertIs(DaemonControl, RuntimeLifecycle)
        self.assertIs(DaemonMode, RuntimeHostRole)

    def test_runtime_transport_closes_owned_handle_once(self) -> None:
        raw = FakeRawCABI()
        transport = CABIRuntimeTransport(CLILibrary(raw), 42, owns_handle=True)
        runtime = RuntimeClient(transport)

        runtime.close()
        runtime.close()

        self.assertEqual(raw.shutdown_handles, [42])

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

    def test_cabi_stream_and_bidi_cancel_are_non_terminal_requests(self) -> None:
        raw = FakeRawCABI()
        runtime = RuntimeClient(CABIRuntimeTransport(CLILibrary(raw), 42, owns_handle=False))

        stream = runtime.invoke_stream(complete_draft())
        stream_cancel = stream.cancel("client stop")

        self.assertEqual(stream_cancel.state, StreamState.CANCEL_REQUESTED)
        self.assertFalse(stream_cancel.cancelled)
        self.assertFalse(stream_cancel.terminal)
        self.assertEqual(stream.state, StreamState.CANCEL_REQUESTED)
        stream_terminal = stream.next()
        self.assertTrue(stream_terminal.terminal)
        self.assertEqual(stream.state, StreamState.TERMINAL_FRAME_SEEN)
        self.assertEqual(raw.stream_cancels, [4001])
        self.assertEqual(raw.stream_closes, [])

        session = runtime.open_bidi(
            complete_draft(),
            (
                BidiStreamDescriptor(
                    stream_id=1, content_type="application/json", ordering="STRICT"
                ),
            ),
        )
        bidi_cancel = session.cancel("client stop")

        self.assertEqual(bidi_cancel.state, BidiState.CANCEL_REQUESTED)
        self.assertFalse(bidi_cancel.terminal)
        self.assertEqual(session.state, BidiState.CANCEL_REQUESTED)
        bidi_terminal = session.receive()
        self.assertTrue(bidi_terminal.terminal)
        self.assertEqual(session.state, BidiState.TERMINAL)
        self.assertEqual(raw.bidi_cancels, [5001])
        self.assertEqual(raw.bidi_closes, [])


if __name__ == "__main__":
    unittest.main()
