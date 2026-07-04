import ctypes
import json
import unittest

from easynet_sdk import (
    AttachOptions,
    BidiFrame,
    BidiState,
    BidiStreamDescriptor,
    Client,
    ConnectOptions,
    DaemonControl,
    DaemonMode,
    AbilityQuery,
    AgentQuery,
    DeviceQuery,
    DirectoryClient,
    DirectoryQueryBase,
    ErrorCode,
    HealthClient,
    IdentityClient,
    InvocationSignature,
    PrepareOptions,
    ReceiptClient,
    ReceiptFetchRequest,
    ResolveQuery,
    RetryHint,
    RuntimeClient,
    SDKError,
    StartConfig,
    StreamState,
    is_code,
)
from easynet_sdk._cabi import (
    CABIDirectoryTransport,
    CABIDiscoveryTransport,
    CABIDaemonTransport,
    CABIIdentityTransport,
    CABIReceiptTransport,
    CABIRuntimeConnector,
    CABIRuntimeTransport,
    CLILibrary,
    EXPECTED_ABI_VERSION,
    _JSON_HANDLE_OUTPUT_SYMBOLS,
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
        self.profile_requests: list[tuple[str, int, object]] = []
        self.prepared_frees: list[int] = []
        self.signed_frees: list[int] = []
        self.handle_frees: list[tuple[int, int]] = []
        self.stream_closes: list[int] = []
        self.stream_cancels: list[int] = []
        self.bidi_sends: list[dict[str, object]] = []
        self.bidi_close_sends: list[int] = []
        self.bidi_closes: list[int] = []
        self.bidi_cancels: list[int] = []
        self.callback_buffers: list[ctypes.Array[ctypes.c_char]] = []
        self.daemon_starts: list[dict[str, object]] = []
        self.daemon_attaches: list[dict[str, object]] = []
        self.daemon_discovers: list[dict[str, object]] = []
        self.daemon_stops: list[int] = []
        self.daemon_detaches: list[int] = []
        self.daemon_open_clients: list[int] = []
        self.stream_events = [
            b'{"sequence":1,"kind":"chunk","state":"Open","terminal":false,'
            b'"payload_json":{"step":1}}',
            b'{"sequence":2,"kind":"terminal","state":"Completed","terminal":true}',
        ]
        self.bidi_frames = [
            b'{"sequence":1,"kind":"terminal","stream_id":1,"terminal":true}'
        ]
        self.prepare_payload = PREPARED_FIXTURE
        self.easynet_abi_version = FakeSymbol(lambda: EXPECTED_ABI_VERSION)
        self.easynet_string_free = FakeSymbol(self._free)
        self.easynet_feature_discovery = FakeSymbol(self._feature_discovery)
        self.easynet_last_error_json = FakeSymbol(self._last_error_json)
        self.easynet_error_json = FakeSymbol(self._error_json)
        self.easynet_init = FakeSymbol(self._init)
        self.easynet_shutdown = FakeSymbol(self._shutdown)
        self.easynet_daemon_start = FakeSymbol(self._daemon_start)
        self.easynet_daemon_attach = FakeSymbol(self._daemon_attach)
        self.easynet_daemon_discover = FakeSymbol(self._daemon_discover)
        self.easynet_daemon_stop = FakeSymbol(self._daemon_stop)
        self.easynet_daemon_detach = FakeSymbol(self._daemon_detach)
        self.easynet_daemon_status = FakeSymbol(self._daemon_status)
        self.easynet_daemon_endpoints = FakeSymbol(self._daemon_endpoints)
        self.easynet_daemon_open_client = FakeSymbol(self._daemon_open_client)
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
        for symbol in _JSON_HANDLE_OUTPUT_SYMBOLS:
            setattr(
                self,
                symbol,
                FakeSymbol(
                    lambda handle, raw, out_ptr, symbol=symbol: self._profile_call(
                        symbol, handle, raw, out_ptr
                    )
                ),
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
        return self._write(out_ptr, DAEMON_CABI_STATUS)

    def _daemon_stop(self, daemon_handle) -> int:
        self.daemon_stops.append(int(daemon_handle.value))
        return 0

    def _daemon_detach(self, daemon_handle) -> int:
        self.daemon_detaches.append(int(daemon_handle.value))
        return 0

    def _daemon_status(self, daemon_handle, out_ptr) -> int:
        return self._write(out_ptr, DAEMON_CABI_STATUS)

    def _daemon_endpoints(self, daemon_handle, out_ptr) -> int:
        return self._write(
            out_ptr,
            b'{"control_endpoint":"unix:///tmp/control.sock",'
            b'"invocation_endpoint":"unix:///tmp/daemon.sock",'
            b'"public_endpoint":null}',
        )

    def _daemon_open_client(self, daemon_handle, out_handle) -> int:
        self.daemon_open_clients.append(int(daemon_handle.value))
        out_handle._obj.value = 808
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
        output_json = self._invocation_output_json(draft)
        return self._write(
            out_ptr,
            json.dumps(
                {
                    "ok": True,
                    "tuple": draft,
                    "terminal_state": "Completed",
                    "output_content_type": "application/json",
                    "output_base64": "e30=",
                    "output_json": output_json,
                    "elapsed_ms": 7,
                    "receipt": None,
                    "error": None,
                },
                separators=(",", ":"),
                sort_keys=True,
            ).encode("utf-8"),
        )

    def _invocation_output_json(self, draft: dict[str, object]) -> dict[str, object]:
        metadata = draft.get("metadata")
        system_ability = (
            metadata.get("system_ability") if isinstance(metadata, dict) else None
        )
        if system_ability == "node.list":
            return {"nodes": [{"node_id": "dev-a", "state": "online"}]}
        if system_ability == "agent.list":
            return {
                "agents": [
                    {
                        "name": "codex",
                        "ura": "easynet:///r/example/agent/alice.codex",
                        "runtime": "codex",
                        "root_exists": True,
                    }
                ]
            }
        if system_ability == "meta.list_abilities":
            return {
                "abilities": [
                    {
                        "name": "fs.read",
                        "ability_ura": "easynet:///r/example/ability/device.dev-a.fs.read",
                        "owner_ura": "easynet:///r/example/device/dev-a",
                    }
                ]
            }
        if system_ability == "namespace.resolve":
            return {
                "answerKind": "RESOLVE_ANSWER_KIND_FINAL_ROUTE",
                "canonicalName": "easynet:///r/example/device/dev-a",
                "ownerUra": "easynet:///r/example/device/dev-a",
                "abilityUra": "easynet:///r/example/ability/device.dev-a.agent.list",
                "records": [],
            }
        return {}

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

    def _invoke_callback(self, callback, user_data, payload: bytes) -> None:
        buffer = ctypes.create_string_buffer(payload)
        self.callback_buffers.append(buffer)
        callback(user_data, ctypes.cast(buffer, ctypes.c_void_p))

    def _invocation_stream_open(
        self, handle, raw, callback, user_data, out_stream_id
    ) -> int:
        self.runtime_requests.append(
            ("stream_open", json.loads(raw.value.decode("utf-8")))
        )
        out_stream_id._obj.value = 404
        for event in self.stream_events:
            self._invoke_callback(callback, user_data, event)
        return 0

    def _invocation_stream_cancel(self, handle, stream_id) -> int:
        self.stream_cancels.append(int(stream_id.value))
        return 0

    def _invocation_stream_close(self, handle, stream_id) -> int:
        self.stream_closes.append(int(stream_id.value))
        return 0

    def _invocation_bidi_open(
        self, handle, raw, callback, user_data, out_bidi_id
    ) -> int:
        self.runtime_requests.append(("bidi_open", json.loads(raw.value.decode("utf-8"))))
        out_bidi_id._obj.value = 505
        for frame in self.bidi_frames:
            self._invoke_callback(callback, user_data, frame)
        return 0

    def _invocation_bidi_send(self, handle, bidi_id, frame_json) -> int:
        self.bidi_sends.append(json.loads(frame_json.value.decode("utf-8")))
        return 0

    def _invocation_bidi_close_send(self, handle, bidi_id) -> int:
        self.bidi_close_sends.append(int(bidi_id.value))
        return 0

    def _invocation_bidi_close(self, handle, bidi_id) -> int:
        self.bidi_closes.append(int(bidi_id.value))
        return 0

    def _invocation_bidi_cancel(self, handle, bidi_id) -> int:
        self.bidi_cancels.append(int(bidi_id.value))
        return 0

    def _profile_call(self, symbol: str, handle, raw, out_ptr) -> int:
        request = json.loads(raw.value.decode("utf-8"))
        self.profile_requests.append((symbol, int(handle.value), request))
        return self._write(out_ptr, self._profile_payload(symbol))

    def _profile_payload(self, symbol: str) -> bytes:
        if symbol == "easynet_directory_build_list_devices_invocation":
            return DIRECTORY_LIST_DEVICES_INVOCATION
        if symbol == "easynet_directory_build_list_agents_invocation":
            return DIRECTORY_LIST_AGENTS_INVOCATION
        if symbol == "easynet_directory_build_list_abilities_invocation":
            return DIRECTORY_LIST_ABILITIES_INVOCATION
        if symbol == "easynet_directory_build_resolve_invocation":
            return DIRECTORY_RESOLVE_INVOCATION
        if symbol == "easynet_receipt_build_fetch_invocation":
            return RECEIPT_FETCH_INVOCATION
        if symbol.endswith("_invocation"):
            return json.dumps(
                complete_draft().to_json_dict(),
                separators=(",", ":"),
                sort_keys=True,
            ).encode("utf-8")
        if symbol == "easynet_publication_build_resource_ref":
            return RESOURCE_REF_PROJECTION
        if symbol == "easynet_publication_validate_package":
            return PACKAGE_VALIDATION_PROJECTION
        if symbol == "easynet_receipt_project":
            return RECEIPT_SUMMARY_PROJECTION
        if symbol == "easynet_directory_project_device_page":
            return DIRECTORY_DEVICE_PAGE_PROJECTION
        if symbol == "easynet_directory_project_agent_page":
            return DIRECTORY_AGENT_PAGE_PROJECTION
        if symbol == "easynet_directory_project_ability_page":
            return DIRECTORY_ABILITY_PAGE_PROJECTION
        if symbol == "easynet_directory_project_resolved_ref":
            return DIRECTORY_RESOLVED_REF_PROJECTION
        if symbol == "easynet_receipt_verify":
            return RECEIPT_VERIFICATION_PROJECTION
        if symbol == "easynet_receipt_verify_chain":
            return RECEIPT_CHAIN_VERIFICATION_PROJECTION
        if symbol == "easynet_receipt_causal_ref":
            return CAUSAL_REF_PROJECTION
        if symbol == "easynet_host_binding_build":
            return HOST_BINDING_PROJECTION
        if symbol == "easynet_host_binding_decode_request":
            return HOST_REQUEST_PROJECTION
        if symbol == "easynet_host_binding_encode_item":
            return HOST_ITEM_FRAME_PROJECTION
        if symbol == "easynet_host_binding_encode_error":
            return HOST_ERROR_FRAME_PROJECTION
        if symbol == "easynet_host_binding_encode_terminal":
            return HOST_TERMINAL_FRAME_PROJECTION
        if symbol == "easynet_host_binding_fold_output_hash":
            return HOST_HASH_STATE_PROJECTION
        if symbol == "easynet_mission_project_status":
            return MISSION_STATUS_PROJECTION
        if symbol == "easynet_mission_project_events":
            return MISSION_EVENT_PAGE_PROJECTION
        if symbol in {
            "easynet_events_project_directory_event",
            "easynet_events_project_terminal",
            "easynet_events_project_drop_report",
        }:
            return EVENT_FRAME_PROJECTION
        if symbol == "easynet_admin_project_gateway_status":
            return GATEWAY_STATUS_PROJECTION
        if symbol == "easynet_admin_project_agent_records":
            return ADMIN_AGENT_PAGE_PROJECTION
        if symbol == "easynet_admin_project_agent_lifecycle_result":
            return ADMIN_RESULT_PROJECTION
        if symbol == "easynet_surface_project_page_record":
            return SURFACE_PAGE_RECORD_PROJECTION
        if symbol == "easynet_surface_project_page_page":
            return SURFACE_PAGE_PAGE_PROJECTION
        if symbol == "easynet_surface_project_manifest":
            return SURFACE_MANIFEST_PROJECTION
        if symbol == "easynet_surface_project_public_page_ref":
            return SURFACE_PUBLIC_REF_PROJECTION
        if symbol == "easynet_surface_project_mutation_result":
            return SURFACE_MUTATION_PROJECTION
        if symbol == "easynet_compatibility_project_model_page":
            return COMPAT_MODEL_PAGE_PROJECTION
        if symbol == "easynet_compatibility_project_chat_completion":
            return COMPAT_CHAT_PROJECTION
        if symbol == "easynet_compatibility_project_chat_stream":
            return COMPAT_STREAM_PROJECTION
        if symbol in {
            "easynet_compatibility_project_file_upload",
            "easynet_compatibility_project_file",
        }:
            return COMPAT_FILE_PROJECTION
        if symbol == "easynet_compatibility_project_file_delete_result":
            return COMPAT_FILE_DELETE_PROJECTION
        if symbol == "easynet_wrappers_project_file_record":
            return WRAPPER_FILE_PROJECTION
        if symbol == "easynet_wrappers_project_terminal_session":
            return WRAPPER_TERMINAL_PROJECTION
        if symbol == "easynet_wrappers_project_remote_desktop_session":
            return WRAPPER_REMOTE_DESKTOP_PROJECTION
        if symbol == "easynet_wrappers_project_browser_session":
            return WRAPPER_BROWSER_PROJECTION
        if symbol == "easynet_wrappers_project_media_session":
            return WRAPPER_MEDIA_PROJECTION
        return b"{}"


DESCRIPTOR_PROJECTION = (
    b'{"kind":"descriptor_ref","valid":true,'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",'
    b'"ability_ura":"easynet:///r/example/ability/device.dev-a.observe.health",'
    b'"descriptor_version":"1.0.0","profile":"easynet-strict-v2",'
    b'"components":{"owner_ura":"easynet:///r/example/device/dev-a"},'
    b'"metadata":{"grammar_owner":"axon"}}'
)

DAEMON_CABI_STATUS = (
    b'{"pid":42,"pid_alive":true,"control_accepting":true,'
    b'"invocation_accepting":true,'
    b'"control_endpoint":"unix:///tmp/control.sock",'
    b'"invocation_endpoint":"unix:///tmp/daemon.sock"}'
)

RESOURCE_REF_PROJECTION = (
    b'{"resource_ura":"easynet:///r/example/resource/device.dev-a/fs/tmp/package",'
    b'"owner_ura":"easynet:///r/example/device/dev-a","namespace":"fs",'
    b'"display_path":"tmp/package","capability":"read",'
    b'"expires_unix_ms":4102444800000,"revision":"fs-local-mapping-v1"}'
)

PACKAGE_VALIDATION_PROJECTION = (
    b'{"profile":"publication","kind":"package_validation","valid":true,'
    b'"package_path":"/tmp/package","manifest_path":"/tmp/package/ability.json",'
    b'"manifest_hash":"sha256:abc","manifest":{"name":"weather",'
    b'"namespace":"er","wire_key":"er.weather","descriptor_version":"1.0.0",'
    b'"description":"Weather","exec_kind":"host_stream",'
    b'"timeout_seconds":null,"input_schema":{"type":"object"},'
    b'"output_schema":null},"errors":[],"metadata":{"profile":"publication"}}'
)

RECEIPT_SUMMARY_PROJECTION = (
    b'{"receipt_ura":null,"invocation_id":"inv-example-1",'
    b'"state":"completed","verified":false,"output":{"ok":true},'
    b'"error":null,"causal_ref":null,"metadata":{}}'
)

RECEIPT_VERIFICATION_PROJECTION = (
    b'{"verified":true,"receipt_ura":"easynet:///r/example/receipt/receipt-1",'
    b'"invocation_id":"inv-example-1","method":"axon-full-receipt",'
    b'"metadata":{"source":"axon"}}'
)

RECEIPT_CHAIN_VERIFICATION_PROJECTION = (
    b'{"verified":false,"continuous":true,'
    b'"method":"daemon_receipt_chain_continuity","reason":"continuity only",'
    b'"requires_full_receipt":true,'
    b'"root_receipt_ura":"easynet:///r/example/receipt/receipt-1",'
    b'"terminal_receipt_ura":"easynet:///r/example/receipt/receipt-2",'
    b'"receipt_count":1,"items":[{"index":0,'
    b'"receipt_ura":"easynet:///r/example/receipt/receipt-1",'
    b'"receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",'
    b'"prev_receipt_hash_hex":null,"continuous":true,"metadata":{}}],'
    b'"metadata":{"chain_projection":"hash_continuity"}}'
)

CAUSAL_REF_PROJECTION = (
    b'{"causal_ref":"receipt:easynet:///r/example/receipt/receipt-1",'
    b'"receipt_ura":"easynet:///r/example/receipt/receipt-1",'
    b'"invocation_id":"inv-example-1","form":"scalar","metadata":{}}'
)

RECEIPT_FETCH_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/'
    b'device.dev-a.invocation.history.get@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"key":{"request_id":"inv-example-1"}},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"receipt",'
    b'"system_ability":"invocation.history.get",'
    b'"carrier_owner":"daemon_sdk"}}'
)

DIRECTORY_LIST_DEVICES_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.node.list@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},"args":{},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"directory_identity","system_ability":"node.list",'
    b'"carrier_owner":"daemon_sdk"}}'
)

DIRECTORY_LIST_AGENTS_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.agent.list@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},"args":{},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"directory_identity","system_ability":"agent.list",'
    b'"carrier_owner":"daemon_sdk"}}'
)

DIRECTORY_LIST_ABILITIES_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.meta.list_abilities@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},"args":{"scope":"local"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"directory_identity",'
    b'"system_ability":"meta.list_abilities","carrier_owner":"daemon_sdk"}}'
)

DIRECTORY_RESOLVE_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.namespace.resolve@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"queryName":"easynet:///r/example/device/dev-a",'
    b'"qtype":"RESOLVE_TYPE_ROUTE"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"directory_identity",'
    b'"system_ability":"namespace.resolve","carrier_owner":"daemon_sdk"}}'
)

DIRECTORY_DEVICE_PAGE_PROJECTION = (
    b'{"profile":"directory_identity","kind":"device_page",'
    b'"item_kind":"device","items":[{"node_id":"dev-a","state":"online",'
    b'"owner_ura":"easynet:///r/example/device/dev-a","metadata":{}}],'
    b'"next_cursor":null,"limit":50,"source":"read_model","metadata":{}}'
)

DIRECTORY_AGENT_PAGE_PROJECTION = (
    b'{"profile":"directory_identity","kind":"agent_page",'
    b'"item_kind":"agent","items":[{"name":"codex",'
    b'"agent_ura":"easynet:///r/example/agent/alice.codex","metadata":{}}],'
    b'"next_cursor":null,"limit":50,"source":"read_model","metadata":{}}'
)

DIRECTORY_ABILITY_PAGE_PROJECTION = (
    b'{"profile":"directory_identity","kind":"ability_page",'
    b'"item_kind":"ability","items":[{"name":"fs.read",'
    b'"ability_ura":"easynet:///r/example/ability/device.dev-a.fs.read",'
    b'"owner_ura":"easynet:///r/example/device/dev-a","metadata":{}}],'
    b'"next_cursor":null,"limit":50,"source":"read_model","metadata":{}}'
)

DIRECTORY_RESOLVED_REF_PROJECTION = (
    b'{"profile":"directory_identity","kind":"resolved_ref",'
    b'"answer_kind":"RESOLVE_ANSWER_KIND_FINAL_ROUTE",'
    b'"query_name":"easynet:///r/example/device/dev-a",'
    b'"canonical_name":"easynet:///r/example/device/dev-a",'
    b'"owner_ura":"easynet:///r/example/device/dev-a",'
    b'"ability_ura":"easynet:///r/example/ability/device.dev-a.agent.list",'
    b'"route_ura":null,"next_hop":null,"selected_route":null,'
    b'"route_candidates":[],"records":[],"negative":null,'
    b'"release_profile":null,"authority":null,"cache_policy":null,'
    b'"metadata":{"source":"namespace.resolve"}}'
)

HOST_BINDING_PROJECTION = (
    b'{"binding_id":"binding-weather-1",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",'
    b'"endpoint":"/tmp/easynet-weather.sock",'
    b'"frame_schema":"host-stream-frame.schema.json",'
    b'"cleanup":{"mode":"unlink_socket"},"timeout_ms":30000,'
    b'"readiness":{"state":"declared","checked":false,"endpoint_ready":null},'
    b'"lifecycle":{"endpoint_owner":"product_host","process_owner":"product_host",'
    b'"frame_contract_owner":"daemon_sdk"},'
    b'"metadata":{"profile":"host_binding"}}'
)

HOST_REQUEST_PROJECTION = (
    b'{"function":"weather.stream","args":{"city":"Singapore"},'
    b'"call_id":"call-weather-1","caller":"easynet:///r/example/user/alice",'
    b'"metadata":{"wire":"host_stream_request_v1"}}'
)

HOST_ITEM_FRAME_PROJECTION = (
    b'{"frame_type":"item","seq":0,"value":{"token":"hello"},'
    b'"error":null,"terminal":null,"output_hash":null}'
)

HOST_ERROR_FRAME_PROJECTION = (
    b'{"frame_type":"error","seq":null,"value":null,'
    b'"error":{"code":"InvalidArgument","stage":"host","message":"bad input",'
    b'"retry":"never","details":{}},"terminal":null,"output_hash":null}'
)

HOST_TERMINAL_FRAME_PROJECTION = (
    b'{"frame_type":"terminal","seq":1,"value":null,"error":null,'
    b'"terminal":{"output_hash":"sha256:abc","frames":1,"metadata":{}},'
    b'"output_hash":"sha256:abc"}'
)

HOST_HASH_STATE_PROJECTION = (
    b'{"algorithm":"sha256(prev_hash || seq_be || canonical_json(value))",'
    b'"output_hash":"sha256:abc","frames":1,"last_seq":0,'
    b'"canonical_json":"{\\"token\\":\\"hello\\"}"}'
)

MISSION_STATUS_PROJECTION = (
    b'{"profile":"mission","kind":"mission_status",'
    b'"mission_id":"mission-1","state":"completed","terminal":true,'
    b'"partial_failures":0,"cancelled":false,"parent_invocation_id":null,'
    b'"parent_receipt_ura":null,"parent_invocation":{},'
    b'"child_invocations":[],"child_receipts":[],"output_refs":[],'
    b'"metadata":{"profile":"mission"}}'
)

MISSION_EVENT_PAGE_PROJECTION = (
    b'{"profile":"mission","kind":"mission_event_page",'
    b'"mission_id":"mission-1","cursor_sequence":0,"next_cursor_sequence":1,'
    b'"has_more":false,"dropped_count":0,"events":[{"profile":"mission",'
    b'"kind":"mission_event","mission_id":"mission-1","sequence":1,'
    b'"event_type":"completed","occurred_unix_ms":1000,"terminal":true,'
    b'"payload":{},"receipt":{},"metadata":{}}],"metadata":{}}'
)

EVENT_FRAME_PROJECTION = (
    b'{"profile":"events","stream":"directory","sequence":1,'
    b'"event_type":"upsert","occurred_unix_ms":1000,"cursor":'
    b'{"stream":"directory","sequence":1,"token":"directory:1"},'
    b'"payload":{},"dropped_count":0,"terminal":false,"metadata":{}}'
)

GATEWAY_STATUS_PROJECTION = (
    b'{"profile":"admin_gateway","kind":"gateway_status","state":"ready",'
    b'"ready":true,"public_endpoint":"https://hub.example",'
    b'"listeners":[],"checks":[],"metadata":{}}'
)

ADMIN_AGENT_PAGE_PROJECTION = (
    b'{"profile":"admin_gateway","kind":"agent_page","items":[],'
    b'"next_cursor":null,"limit":50,"metadata":{}}'
)

ADMIN_RESULT_PROJECTION = (
    b'{"profile":"admin_gateway","kind":"admin_result","operation":"agent_start",'
    b'"state":"completed","ok":true,"metadata":{}}'
)

SURFACE_PAGE_RECORD_PROJECTION = (
    b'{"profile":"surface","kind":"page_record","page_id":"page-1",'
    b'"owner_ura":"easynet:///r/example/device/dev-a","project_id":"proj",'
    b'"folder":"/tmp/site","visibility":"public","state":"ready",'
    b'"metadata":{}}'
)

SURFACE_PAGE_PAGE_PROJECTION = (
    b'{"profile":"surface","kind":"page_page","item_kind":"page_record",'
    b'"items":[],"next_cursor":null,"limit":50,"source":"pages_read_model",'
    b'"metadata":{}}'
)

SURFACE_MANIFEST_PROJECTION = (
    b'{"profile":"surface","kind":"surface_manifest","page_id":"page-1",'
    b'"owner_ura":"easynet:///r/example/device/dev-a","surface_ref":"surface:page-1",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.surface.page@1.0.0",'
    b'"descriptor_version":"1.0.0","files":[],"routes":[],"metadata":{}}'
)

SURFACE_PUBLIC_REF_PROJECTION = (
    b'{"profile":"surface","kind":"public_page_ref","page_id":"page-1",'
    b'"owner_ura":"easynet:///r/example/device/dev-a",'
    b'"surface_ref":"surface:page-1","public_ref":"https://hub.example/page-1",'
    b'"route_kind":"hub_web","metadata":{}}'
)

SURFACE_MUTATION_PROJECTION = (
    b'{"profile":"surface","kind":"surface_mutation_result","operation":"delete",'
    b'"page_id":"page-1","removed":true,"state":"deleted","metadata":{}}'
)

COMPAT_MODEL_PAGE_PROJECTION = (
    b'{"profile":"compatibility","kind":"model_page","object":"list",'
    b'"data":[],"metadata":{}}'
)

COMPAT_CHAT_PROJECTION = (
    b'{"profile":"compatibility","kind":"chat_completion","id":"chatcmpl-1",'
    b'"object":"chat.completion","created":1000,"model":"model-1",'
    b'"choices":[],"usage":null,"metadata":{}}'
)

COMPAT_STREAM_PROJECTION = (
    b'{"profile":"compatibility","kind":"chat_completion_stream",'
    b'"chunks":[],"metadata":{}}'
)

COMPAT_FILE_PROJECTION = (
    b'{"profile":"compatibility","kind":"file","id":"file-1","object":"file",'
    b'"bytes":1,"created_at":1000,"filename":"input.json","purpose":"assistants",'
    b'"status":"processed","metadata":{}}'
)

COMPAT_FILE_DELETE_PROJECTION = (
    b'{"profile":"compatibility","kind":"file_delete_result","id":"file-1",'
    b'"object":"file","deleted":true,"metadata":{}}'
)

WRAPPER_FILE_PROJECTION = (
    b'{"profile":"wrappers","kind":"file_record","file_id":"file-1",'
    b'"owner_ura":"easynet:///r/example/device/dev-a","resource_ura":"res",'
    b'"state":"ready","metadata":{}}'
)

WRAPPER_TERMINAL_PROJECTION = (
    b'{"profile":"wrappers","kind":"terminal_session","session_id":"term-1",'
    b'"owner_ura":"easynet:///r/example/device/dev-a","state":"ready",'
    b'"pty_ref":"pty:1","metadata":{}}'
)

WRAPPER_REMOTE_DESKTOP_PROJECTION = (
    b'{"profile":"wrappers","kind":"remote_desktop_session","session_id":"rdp-1",'
    b'"owner_ura":"easynet:///r/example/device/dev-a","state":"ready",'
    b'"desktop_ref":"desktop:1","metadata":{}}'
)

WRAPPER_BROWSER_PROJECTION = (
    b'{"profile":"wrappers","kind":"browser_session","session_id":"browser-1",'
    b'"owner_ura":"easynet:///r/example/device/dev-a","state":"ready",'
    b'"browser_ref":"browser:1","metadata":{}}'
)

WRAPPER_MEDIA_PROJECTION = (
    b'{"profile":"wrappers","kind":"media_session","session_id":"media-1",'
    b'"owner_ura":"easynet:///r/example/device/dev-a","state":"ready",'
    b'"media_kind":"audio","stream_ref":"stream:1","metadata":{}}'
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
        owner_descriptor_ref = client.owner_ability_descriptor_ref(
            "easynet:///r/example/device/dev-a",
            "observe.health",
            "1.0.0",
        )

        self.assertEqual(
            ability_ura, "easynet:///r/example/ability/device.dev-a.observe.health"
        )
        self.assertEqual(owner_ura, "easynet:///r/example/device/dev-a")
        self.assertEqual(
            descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        )
        self.assertEqual(owner_descriptor_ref, descriptor_ref)
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

    def test_daemon_transport_discovers_start_attaches_and_opens_runtime(
        self,
    ) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        transport = CABIDaemonTransport(lib)
        control = DaemonControl(transport)

        endpoints = control.discover()
        started = control.start(
            StartConfig(
                mode=DaemonMode.DEVICE,
                device_id="dev-a",
                daemon_bin="/tmp/easynet-daemon",
                log_path="/tmp/easynet.log",
                detached=True,
            )
        )
        attached = control.attach(
            AttachOptions(control_endpoint="unix:///tmp/control.sock")
        )
        runtime = started.open_runtime(ConnectOptions(max_message_bytes=4096))
        started.stop()
        attached.detach()

        self.assertEqual(endpoints.invocation_endpoint, "unix:///tmp/daemon.sock")
        self.assertEqual(started.handle_id, "606")
        self.assertEqual(attached.handle_id, "707")
        self.assertEqual(raw.daemon_starts[0]["node_id"], "dev-a")
        self.assertEqual(raw.daemon_starts[0]["detach"], True)
        self.assertNotIn("device_id", raw.daemon_starts[0])
        self.assertNotIn("detached", raw.daemon_starts[0])
        self.assertEqual(raw.daemon_open_clients, [606])
        self.assertEqual(raw.daemon_stops, [606])
        self.assertEqual(raw.daemon_detaches, [707])
        self.assertIsNotNone(runtime)

    def test_runtime_connector_resolves_handshakes_detaches_and_closes(
        self,
    ) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        connector = CABIRuntimeConnector(lib)

        endpoint_json = connector.resolve(
            ConnectOptions(control_path="/tmp/control.json").to_json_bytes()
        )
        endpoint = json.loads(endpoint_json.decode("utf-8"))
        runtime, facts_json = connector.handshake(endpoint_json)
        facts = json.loads(facts_json.decode("utf-8"))
        connector.close()
        connector.close()

        self.assertIsInstance(runtime, CABIRuntimeTransport)
        self.assertEqual(endpoint["endpoint"], "unix:///tmp/daemon.sock")
        self.assertEqual(endpoint["control_path"], "/tmp/control.json")
        self.assertEqual(facts["transport"], "c_abi")
        self.assertTrue(facts["ready"])
        self.assertEqual(raw.daemon_discovers, [{"control_path": "/tmp/control.json"}])
        self.assertEqual(
            raw.daemon_attaches,
            [
                {
                    "control_endpoint": "unix:///tmp/control.sock",
                    "control_path": "/tmp/control.json",
                    "invocation_endpoint": "unix:///tmp/daemon.sock",
                }
            ],
        )
        self.assertEqual(raw.daemon_open_clients, [707])
        self.assertEqual(raw.daemon_detaches, [707])
        self.assertEqual(raw.shutdown_handles, [808])

    def test_runtime_connector_preserves_explicit_endpoint_override(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        connector = CABIRuntimeConnector(lib)

        endpoint_json = connector.resolve(
            ConnectOptions(
                endpoint="unix:///tmp/explicit-daemon.sock",
                control_path="/tmp/control.json",
            ).to_json_bytes()
        )
        connector.handshake(endpoint_json)
        connector.close()

        self.assertEqual(
            raw.daemon_attaches[0]["invocation_endpoint"],
            "unix:///tmp/explicit-daemon.sock",
        )

    def test_runtime_connector_detaches_after_open_runtime_failure(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)

        def fail_open_client(_daemon_handle):
            raise SDKError(
                code=ErrorCode.DAEMON_OFFLINE,
                stage="cabi",
                retry=RetryHint.SAFE,
                message="daemon open client failed",
            )

        lib.daemon_open_client = fail_open_client
        connector = CABIRuntimeConnector(lib)
        endpoint_json = connector.resolve(
            ConnectOptions(control_path="/tmp/control.json").to_json_bytes()
        )

        with self.assertRaises(SDKError) as caught:
            connector.handshake(endpoint_json)

        self.assertTrue(is_code(caught.exception, ErrorCode.DAEMON_OFFLINE))
        self.assertEqual(raw.daemon_attaches[0]["control_path"], "/tmp/control.json")
        self.assertEqual(raw.daemon_detaches, [707])

    def test_daemon_transport_rejects_unsupported_start_fields(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        control = DaemonControl(CABIDaemonTransport(lib))

        with self.assertRaises(SDKError) as caught:
            control.start(
                StartConfig(
                    mode=DaemonMode.HUB,
                    listen_tcp="127.0.0.1:9443",
                    tls_cert_path="/tmp/cert.pem",
                    tls_key_path="/tmp/key.pem",
                )
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.NOT_IMPLEMENTED))
        self.assertEqual(raw.daemon_starts, [])

    def test_daemon_transport_rejects_unknown_handle(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        transport = CABIDaemonTransport(lib)

        with self.assertRaises(SDKError) as caught:
            transport.status("missing")

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_HANDLE))

    def test_daemon_transport_close_detaches_owned_handles_once(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        transport = CABIDaemonTransport(lib)
        control = DaemonControl(transport)

        attached = control.attach(
            AttachOptions(control_endpoint="unix:///tmp/control.sock")
        )
        transport.close()
        transport.close()

        self.assertEqual(attached.handle_id, "707")
        self.assertEqual(raw.daemon_detaches, [707])
        with self.assertRaises(SDKError) as caught:
            transport.discover(b"{}")
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

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

    def test_receipt_fetch_uses_carrier_invoke_and_projection(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = ReceiptClient(CABIReceiptTransport(lib, handle=7))

        summary = client.fetch(
            ReceiptFetchRequest(
                caller_ura="easynet:///r/example/agent/alice.sdk",
                callee_ura="easynet:///r/example/device/dev-a",
                descriptor_ref=(
                    "easynet:///r/example/ability/"
                    "device.dev-a.invocation.history.get@1.0.0"
                ),
                subject_ura="easynet:///r/example/device/dev-a",
                descriptor_version="1.0.0",
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                request_id="inv-example-1",
            )
        )

        self.assertEqual(summary.state, "completed")
        self.assertEqual(summary.output, {"ok": True})
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            ["easynet_receipt_build_fetch_invocation", "easynet_receipt_project"],
        )
        self.assertEqual(raw.profile_requests[0][1], 7)
        self.assertEqual(
            raw.profile_requests[0][2]["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
        )
        self.assertEqual(raw.profile_requests[0][2]["request_id"], "inv-example-1")
        self.assertEqual(raw.runtime_requests[0][0], "invoke")
        self.assertEqual(
            raw.runtime_requests[0][1]["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
        )
        self.assertEqual(
            raw.runtime_requests[0][1]["metadata"]["system_ability"],
            "invocation.history.get",
        )
        self.assertEqual(raw.profile_requests[1][2]["terminal_state"], "Completed")
        self.assertEqual(raw.buffers, {})

    def test_directory_live_methods_use_carrier_invoke_and_projection(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = DirectoryClient(CABIDirectoryTransport(lib, handle=7))
        base = DirectoryQueryBase(
            caller_ura="easynet:///r/example/agent/alice.sdk",
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="easynet:///r/example/device/dev-a",
            descriptor_version="1.0.0",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context={"form": "none"},
        )

        devices = client.list_devices(DeviceQuery(base))
        agents = client.list_agents(AgentQuery(base))
        abilities = client.list_abilities(AbilityQuery(base, scope="local"))
        resolved = client.resolve(
            ResolveQuery(
                base=base,
                query_name="easynet:///r/example/device/dev-a",
                qtype="route",
            )
        )

        self.assertEqual(devices.items[0]["node_id"], "dev-a")
        self.assertEqual(agents.items[0]["name"], "codex")
        self.assertEqual(abilities.items[0]["name"], "fs.read")
        self.assertEqual(
            resolved.ability_ura,
            "easynet:///r/example/ability/device.dev-a.agent.list",
        )
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            [
                "easynet_directory_build_list_devices_invocation",
                "easynet_directory_project_device_page",
                "easynet_directory_build_list_agents_invocation",
                "easynet_directory_project_agent_page",
                "easynet_directory_build_list_abilities_invocation",
                "easynet_directory_project_ability_page",
                "easynet_directory_build_resolve_invocation",
                "easynet_directory_project_resolved_ref",
            ],
        )
        self.assertEqual(
            [item[1]["metadata"]["system_ability"] for item in raw.runtime_requests],
            ["node.list", "agent.list", "meta.list_abilities", "namespace.resolve"],
        )
        self.assertEqual(
            raw.profile_requests[1][2]["output_json"]["nodes"][0]["node_id"],
            "dev-a",
        )
        self.assertEqual(
            raw.profile_requests[3][2]["output_json"]["agents"][0]["name"],
            "codex",
        )
        self.assertEqual(
            raw.profile_requests[5][2]["output_json"]["abilities"][0]["name"],
            "fs.read",
        )
        self.assertEqual(
            raw.profile_requests[7][2]["output_json"]["answerKind"],
            "RESOLVE_ANSWER_KIND_FINAL_ROUTE",
        )
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

    def test_runtime_transport_stream_callbacks_drive_stream_handle(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = RuntimeClient(CABIRuntimeTransport(lib, handle=7))

        stream = client.invoke_stream(complete_draft())
        first = stream.next()
        terminal = stream.next()
        stream.close()

        self.assertEqual(stream.stream_id, "404")
        self.assertEqual(first.payload_json, {"step": 1})
        self.assertTrue(terminal.terminal)
        self.assertEqual(stream.state, StreamState.CLOSED)
        self.assertEqual(raw.stream_closes, [404])
        self.assertEqual(raw.stream_cancels, [])

    def test_runtime_transport_stream_timeout_keeps_inbox_open(self) -> None:
        raw = FakeRawCABI()
        raw.stream_events = []
        lib = CLILibrary(raw)
        client = RuntimeClient(CABIRuntimeTransport(lib, handle=7))

        stream = client.invoke_stream(complete_draft())
        with self.assertRaises(TimeoutError):
            stream.next(timeout=0.001)

        self.assertEqual(stream.state, StreamState.OPEN)
        self.assertEqual(raw.stream_closes, [])
        self.assertEqual(raw.stream_cancels, [])
        stream.close()
        self.assertEqual(raw.stream_closes, [404])

    def test_runtime_transport_stream_cancel_is_terminal(self) -> None:
        raw = FakeRawCABI()
        raw.stream_events = []
        lib = CLILibrary(raw)
        client = RuntimeClient(CABIRuntimeTransport(lib, handle=7))

        stream = client.invoke_stream(complete_draft())
        outcome = stream.cancel("client stop")

        self.assertTrue(outcome.cancelled)
        self.assertEqual(raw.stream_cancels, [404])

    def test_runtime_transport_stream_callback_overflow_fails_bounded_queue(
        self,
    ) -> None:
        raw = FakeRawCABI()
        raw.stream_events = [
            json.dumps(
                {"sequence": index + 1, "kind": "chunk", "terminal": False},
                separators=(",", ":"),
            ).encode("utf-8")
            for index in range(1025)
        ]
        lib = CLILibrary(raw)
        client = RuntimeClient(CABIRuntimeTransport(lib, handle=7))

        stream = client.invoke_stream(complete_draft())
        with self.assertRaises(SDKError) as caught:
            stream.next()

        self.assertTrue(is_code(caught.exception, ErrorCode.PROTOCOL))
        self.assertEqual(stream.state, StreamState.FAILED)
        stream.close()

    def test_runtime_transport_bidi_callbacks_drive_session(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = RuntimeClient(CABIRuntimeTransport(lib, handle=7))

        session = client.open_bidi(
            complete_draft(),
            (
                BidiStreamDescriptor(
                    stream_id=1,
                    content_type="application/json",
                    ordering="STRICT",
                ),
            ),
        )
        ack = session.send(BidiFrame(sequence=1, kind="data", stream_id=1))
        close_send = session.close_send()
        frame = session.receive()
        session.close()

        self.assertEqual(session.session_id, "505")
        self.assertEqual(ack.sequence, 1)
        self.assertFalse(close_send.terminal)
        self.assertTrue(frame.terminal)
        self.assertEqual(raw.bidi_close_sends, [505])
        self.assertEqual(raw.bidi_closes, [505])
        self.assertEqual(raw.bidi_sends[0]["kind"], "data")
        bidi_open = [item for item in raw.runtime_requests if item[0] == "bidi_open"][0]
        self.assertEqual(
            bidi_open[1]["bidi_streams"],
            [
                {
                    "content_type": "application/json",
                    "ordering": "STRICT",
                    "stream_id": 1,
                }
            ],
        )

    def test_runtime_transport_bidi_timeout_keeps_inbox_open(self) -> None:
        raw = FakeRawCABI()
        raw.bidi_frames = []
        lib = CLILibrary(raw)
        client = RuntimeClient(CABIRuntimeTransport(lib, handle=7))

        session = client.open_bidi(
            complete_draft(),
            (BidiStreamDescriptor(stream_id=1, content_type="application/json"),),
        )
        with self.assertRaises(TimeoutError):
            session.receive(timeout=0.001)

        self.assertEqual(session.state, BidiState.OPEN)
        self.assertEqual(raw.bidi_closes, [])
        self.assertEqual(raw.bidi_cancels, [])
        session.cancel("client stop")
        self.assertEqual(raw.bidi_cancels, [505])

    def test_runtime_transport_bidi_cancel_is_terminal(self) -> None:
        raw = FakeRawCABI()
        raw.bidi_frames = []
        lib = CLILibrary(raw)
        client = RuntimeClient(CABIRuntimeTransport(lib, handle=7))

        session = client.open_bidi(
            complete_draft(),
            (BidiStreamDescriptor(stream_id=1, content_type="application/json"),),
        )
        outcome = session.cancel("client stop")

        self.assertTrue(outcome.terminal)
        self.assertEqual(raw.bidi_cancels, [505])

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

    def test_owned_runtime_transport_closes_active_stream_and_bidi_on_close(
        self,
    ) -> None:
        raw = FakeRawCABI()
        raw.stream_events = []
        raw.bidi_frames = []
        lib = CLILibrary(raw)
        handle = lib.init("")
        transport = CABIRuntimeTransport(lib, handle=handle, owns_handle=True)
        client = RuntimeClient(transport)

        client.invoke_stream(complete_draft())
        client.open_bidi(
            complete_draft(),
            (BidiStreamDescriptor(stream_id=1, content_type="application/json"),),
        )
        client.close()

        self.assertEqual(raw.stream_closes, [404])
        self.assertEqual(raw.bidi_closes, [505])
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
