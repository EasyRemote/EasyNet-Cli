import ctypes
import json
import unittest
from unittest.mock import patch

from easynet_sdk import (
    AttachOptions,
    BidiFrame,
    BidiState,
    BidiStreamDescriptor,
    Client,
    ConnectOptions,
    DaemonControl,
    DaemonMode,
    CompatibilityCarrierBase,
    CompatibilityChatCompletionRequest,
    CompatibilityClient,
    CompatibilityFileDeleteRequest,
    CompatibilityFileRequest,
    CompatibilityFileUploadRequest,
    CompatibilityListModelsRequest,
    CompatibilityStreamChatCompletionRequest,
    AbilityQuery,
    AbilityDeployRequest,
    AbilityImplID,
    AbilityImplLifecycleRequest,
    AgentQuery,
    AdminAgentListRequest,
    AdminAgentRefreshRequest,
    AdminAgentStartRequest,
    AdminAgentStopRequest,
    AdminCarrierBase,
    AdminClient,
    AdminGatewayStatusRequest,
    AdminJoinHubRequest,
    AdminLeaveHubRequest,
    AdminSessionListRequest,
    DeviceQuery,
    CreateDeviceSessionRequest,
    CreatePairingRequest,
    DirectoryClient,
    DirectoryQueryBase,
    DirectorySubscriptionCursor,
    DirectorySubscriptionRequest,
    DeleteDeviceSessionRequest,
    ErrorCode,
    EventClient,
    EventCursor,
    EventsCarrierBase,
    EventsDirectorySubscriptionRequest,
    EventsSessionSubscriptionRequest,
    HealthClient,
    IdentityCarrierBase,
    IdentityClient,
    LocalResourceRefRequest,
    ResourceRef,
    InvocationSignature,
    MissionCancelRequest,
    MissionCarrierBase,
    MissionClient,
    MissionRunFileRequest,
    MissionRunRequest,
    MissionTrackRequest,
    PairingPreflightRequest,
    PrepareOptions,
    PublicationClient,
    PublishedAbilityQuery,
    PublishedAbilityShowRequest,
    ReceiptCarrierBase,
    ReceiptClient,
    ReceiptFetchRequest,
    ReceiptHistoryReadRequest,
    ResolveQuery,
    RetryHint,
    RuntimeClient,
    SDKError,
    RevokeDeviceRequest,
    SignerRequest,
    SigningKeyListRequest,
    SigningKeyRegistrationRequest,
    SigningKeyRevokeRequest,
    StartConfig,
    StreamState,
    SurfaceCarrierBase,
    SurfaceClient,
    SurfaceCreatePageRequest,
    SurfaceDeletePageRequest,
    SurfaceHealthRequest,
    SurfaceListPagesRequest,
    SurfaceManifestRequest,
    UnpublishAbilityRequest,
    ValidatePairingRequest,
    VerifyDeviceCredentialRequest,
    WrapperBrowserSessionRequest,
    WrapperBrowserStartRequest,
    WrapperCarrierBase,
    WrapperClient,
    WrapperFileRecordRequest,
    WrapperFileTransferRequest,
    WrapperMediaSessionRequest,
    WrapperMediaStartRequest,
    WrapperRemoteDesktopSessionRequest,
    WrapperRemoteDesktopStartRequest,
    WrapperTerminalSessionRequest,
    WrapperTerminalStartRequest,
    is_code,
)
from easynet_sdk._cabi import (
    CABIAdminTransport,
    CABICompatibilityTransport,
    CABIDirectoryTransport,
    CABIDiscoveryTransport,
    CABIDaemonTransport,
    CABIEventTransport,
    CABIIdentityTransport,
    CABIMissionTransport,
    CABIPublicationTransport,
    CABIReceiptTransport,
    CABIRuntimeConnector,
    CABIRuntimeTransport,
    CABISurfaceTransport,
    CABIWrapperTransport,
    CLILibrary,
    EXPECTED_ABI_VERSION,
    _JSON_HANDLE_OUTPUT_SYMBOLS,
    open_cabi_admin_transport,
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
        self.init_paths: list[str] = []
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
        self.daemon_invocation_endpoint_calls: list[int] = []
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
        self.easynet_daemon_invocation_endpoint = FakeSymbol(
            self._daemon_invocation_endpoint
        )
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
            b'"symbols":{"directory_identity_projection":true,'
            b'"identity_signing_key_lifecycle":true,'
            b'"events_session_stream":true},"axon_pb":true}',
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

    def _daemon_invocation_endpoint(self, daemon_handle, out_ptr) -> int:
        self.daemon_invocation_endpoint_calls.append(int(daemon_handle.value))
        return self._write(out_ptr, b"unix:///tmp/daemon.sock")

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
            b'"components":{"owner_ura":"easynet:///r/example/device/dev-a",'
            b'"owner_kind":"device","public_name":"observe.health",'
            b'"local_registry_ability":"easynet:///r/example/device/dev-a:observe.health",'
            b'"namespace":"observe","local_name":"health"},'
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
        if system_ability == "agent.start":
            return {
                "agent_ura": "easynet:///r/example/agent/alice.codex",
                "replaced_prior": False,
                "runtime_registered": 1,
                "runtime_failed": 0,
                "ack": True,
            }
        if system_ability == "agent.stop":
            return {
                "ack": True,
                "runtime_removed": 1,
                "runtime_failed": 0,
            }
        if system_ability == "agent.refresh":
            return {
                "agents_scanned": 1,
                "runtime_registered": 1,
                "runtime_failed": 0,
            }
        if system_ability == "session.list":
            return {
                "sessions": [
                    {
                        "id": "dev-session-1",
                        "tenant": "example",
                        "node": "dev-a",
                        "agent": "codex",
                        "started_unix_ms": 1767225600000,
                    }
                ]
            }
        if system_ability == "identity.register_pubkey":
            return {"ok": True}
        if system_ability == "identity.list_user_pubkeys":
            return {
                "agent_ura": "easynet:///r/example/agent/alice.sdk",
                "keys": [
                    {
                        "public_key_b64": "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=",
                        "added_at_unix_ms": 1783100000123,
                    }
                ],
                "rotation_epoch": 3,
                "revoked_key_count": 1,
            }
        if system_ability == "identity.revoke_user_pubkey":
            return {"ok": True, "removed": True}
        if system_ability == "meta.list_abilities":
            args = draft.get("args")
            if (
                isinstance(args, dict)
                and args.get("subject_ura")
                == "easynet:///r/example/ability/device.dev-a.er.weather"
            ):
                return {
                    "abilities": [
                        {
                            "name": "weather",
                            "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                            "owner_ura": "easynet:///r/example/device/dev-a",
                            "version": "1.0.0",
                        }
                    ]
                }
            return {
                "abilities": [
                    {
                        "name": "fs.read",
                        "ability_ura": "easynet:///r/example/ability/device.dev-a.fs.read",
                        "owner_ura": "easynet:///r/example/device/dev-a",
                    }
                ]
            }
        if system_ability == "ability.deploy":
            return {
                "public_name": "weather",
                "namespace": "er",
                "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                "node_id": "local",
                "install_id": "install-1",
                "state": "enabled",
            }
        if system_ability == "ability.unpublish":
            return {
                "ok": True,
                "owner_ura": "easynet:///r/example/device/dev-a",
                "public_name": "weather",
                "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
                "removed_path": "/tmp/easynet/abilities/weather.ability.json",
                "content_hash": "sha256:abc",
            }
        if system_ability in {"ability.impl.enable", "ability.impl.disable"}:
            args = draft.get("args")
            return {
                "ok": True,
                "owner_ura": "easynet:///r/example/device/dev-a",
                "ability_ura": (
                    args.get("ability_ura")
                    if isinstance(args, dict)
                    else "easynet:///r/example/ability/device.dev-a.er.weather"
                ),
                "impl_id": args.get("impl_id") if isinstance(args, dict) else "impl-1",
            }
        if system_ability in {"mission.run", "mission.track", "mission.cancel"}:
            return {
                "run_id": "mission-1",
                "running": False,
                "meta": {
                    "trace_id": "mission-1",
                    "status": "cancelled"
                    if system_ability == "mission.cancel"
                    else "ok",
                    "steps_failed": 0,
                },
            }
        if system_ability == "invocation.history.list":
            return {
                "records": [
                    {"invocation_id": "inv-example-1", "state": "completed"}
                ],
                "next_cursor": None,
            }
        if system_ability == "invocation.history.get":
            return {
                "record": {"invocation_id": "inv-example-1", "state": "completed"}
            }
        if system_ability == "invocation.trace.get":
            return {
                "trace_id": "trace-1",
                "nodes": [],
                "edges": [],
                "edge_semantics": "Axon causal links",
            }
        if system_ability == "pages.list":
            return {
                "projects": [
                    {
                        "page_id": "docs",
                        "owner_ura": "easynet:///r/example/agent/alice.pages",
                        "surface_ref": "easynet:///r/example/resource/alice.docs",
                        "public_ref": "https://example/web/alice/docs/",
                        "status": "published",
                        "metadata": {"project_id": "docs"},
                    }
                ]
            }
        if system_ability in {"pages.publish", "pages.get"}:
            return {
                "page_id": "docs",
                "owner_ura": "easynet:///r/example/agent/alice.pages",
                "surface_ref": "easynet:///r/example/resource/alice.docs",
                "public_ref": "https://example/web/alice/docs/",
                "status": "published",
                "metadata": {"project_id": "docs"},
            }
        if system_ability == "pages.unpublish":
            return {"project_id": "docs", "removed": True}
        if system_ability == "pages.health":
            return {
                "state": "ready",
                "ready": True,
                "owner_ura": "easynet:///r/example/agent/alice.pages",
                "surface_ref": "easynet:///r/example/resource/alice.docs",
                "descriptor_version": "1.0.0",
                "page_count": 1,
                "checks": [
                    {
                        "name": "manifest",
                        "state": "ready",
                        "ready": True,
                        "latency_ms": 3,
                        "metadata": {"source": "pages.get"},
                    }
                ],
            }
        if system_ability == "openai.list_models":
            return {
                "object": "list",
                "data": [
                    {
                        "id": "easynet:///r/example/ability/alice.codex.chat",
                        "object": "model",
                        "created": 0,
                        "owned_by": "easynet",
                        "ability": "codex.chat",
                    }
                ],
            }
        if system_ability == "openai.chat_completions":
            return {
                "id": "chatcmpl-example",
                "object": "chat.completion",
                "created": 1,
                "model": "easynet:///r/example/ability/alice.codex.chat",
                "choices": [
                    {
                        "index": 0,
                        "message": {"role": "assistant", "content": "ok"},
                        "finish_reason": "stop",
                    }
                ],
                "usage": {
                    "prompt_tokens": 3,
                    "completion_tokens": 1,
                    "total_tokens": 4,
                },
            }
        if system_ability in {"openai.files.upload", "openai.files.retrieve"}:
            return {
                "id": "file-easynet-docs-1",
                "file_ref": "easynet:///r/example/resource/alice.files/prompt.jsonl",
                "filename": "prompt.jsonl",
                "purpose": "batch",
                "bytes": 19,
                "created_at": 1783094400,
                "status": "processed",
                "owner_ura": "easynet:///r/example/agent/alice.sdk",
                "content_type": "application/jsonl",
            }
        if system_ability == "openai.files.delete":
            return {"id": "file-easynet-docs-1", "deleted": True}
        if system_ability == "wrapper.file.transfer":
            return {
                "file_ref": "easynet:///r/example/resource/alice.files/report.txt",
                "owner_ura": "easynet:///r/example/agent/alice.sdk",
                "content_type": "text/plain",
                "size_bytes": 42,
                "content_hash": (
                    "sha256:"
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                ),
                "metadata": {"route": "upload"},
            }
        if system_ability == "wrapper.terminal.start":
            return {
                "session_id": "term-1",
                "owner_ura": "easynet:///r/example/agent/alice.sdk",
                "state": "ready",
                "terminal_ref": "pty:1",
            }
        if system_ability == "wrapper.remote_desktop.start":
            return {
                "session_id": "rdp-1",
                "owner_ura": "easynet:///r/example/agent/alice.sdk",
                "state": "ready",
                "display_ref": "display:1",
            }
        if system_ability == "wrapper.browser.start":
            return {
                "session_id": "browser-1",
                "owner_ura": "easynet:///r/example/agent/alice.sdk",
                "state": "ready",
                "browser_ref": "browser:1",
            }
        if system_ability == "wrapper.media.start":
            return {
                "session_id": "media-1",
                "owner_ura": "easynet:///r/example/agent/alice.sdk",
                "state": "ready",
                "media_kind": "voice",
                "stream_ref": "stream:1",
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
        if symbol in {
            "easynet_publication_build_enable_ability_impl_invocation",
            "easynet_publication_build_disable_ability_impl_invocation",
        }:
            required = {
                "caller_ura",
                "callee_ura",
                "subject_ura",
                "descriptor_version",
                "nonce_base64",
                "causal_context",
                "impl_id",
                "ability_ura",
            }
            if not required.issubset(request):
                self.last_error_json = (
                    b'{"code":"INVALID_ARGUMENT","stage":"cabi",'
                    b'"message":"complete ability impl lifecycle invocation carrier is required",'
                    b'"retry":"never","source":"cabi","details":{}}'
                )
                return 11
        return self._write(out_ptr, self._profile_payload(symbol, request))

    def _profile_payload(self, symbol: str, request: object | None = None) -> bytes:
        if symbol == "easynet_identity_build_register_signing_key_invocation":
            return IDENTITY_REGISTER_SIGNING_KEY_INVOCATION
        if symbol == "easynet_identity_build_list_signing_keys_invocation":
            return IDENTITY_LIST_SIGNING_KEYS_INVOCATION
        if symbol == "easynet_identity_build_revoke_signing_key_invocation":
            return IDENTITY_REVOKE_SIGNING_KEY_INVOCATION
        if symbol == "easynet_identity_project_signing_key_record":
            return IDENTITY_SIGNING_KEY_RECORD
        if symbol == "easynet_identity_project_signing_key_page":
            return IDENTITY_SIGNING_KEY_PAGE
        if symbol == "easynet_identity_project_signing_key_revoke_result":
            return IDENTITY_SIGNING_KEY_REVOKE
        if symbol == "easynet_identity_project_signer_handle":
            return IDENTITY_SIGNER_HANDLE
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
        if symbol == "easynet_receipt_build_list_history_invocation":
            return RECEIPT_LIST_HISTORY_INVOCATION
        if symbol == "easynet_receipt_build_get_history_invocation":
            return RECEIPT_GET_HISTORY_INVOCATION
        if symbol == "easynet_receipt_build_trace_invocation":
            return RECEIPT_TRACE_INVOCATION
        if symbol == "easynet_events_build_directory_subscription_invocation":
            return EVENTS_DIRECTORY_SUBSCRIPTION_INVOCATION
        if symbol == "easynet_events_build_session_subscription_invocation":
            return EVENTS_SESSION_SUBSCRIPTION_INVOCATION
        if symbol == "easynet_publication_build_deploy_invocation":
            return PUBLICATION_DEPLOY_INVOCATION
        if symbol == "easynet_publication_project_deploy_result":
            return PUBLICATION_DEPLOY_RESULT_PROJECTION
        if symbol == "easynet_publication_install_plugin":
            return PUBLICATION_PLUGIN_INSTALL_PROJECTION
        if symbol == "easynet_publication_build_list_abilities_invocation":
            return PUBLICATION_LIST_INVOCATION
        if symbol == "easynet_publication_project_ability_page":
            return PUBLICATION_ABILITY_PAGE
        if symbol == "easynet_publication_build_show_ability_invocation":
            return PUBLICATION_SHOW_INVOCATION
        if symbol == "easynet_publication_project_ability_record":
            return PUBLICATION_ABILITY_RECORD
        if symbol == "easynet_publication_build_unpublish_invocation":
            return PUBLICATION_UNPUBLISH_INVOCATION
        if symbol == "easynet_publication_project_unpublish_result":
            return PUBLICATION_UNPUBLISH_RESULT_PROJECTION
        if symbol == "easynet_publication_build_enable_ability_impl_invocation":
            return PUBLICATION_ENABLE_IMPL_INVOCATION
        if symbol == "easynet_publication_project_enable_ability_impl_result":
            return PUBLICATION_ENABLE_IMPL_PROJECTION
        if symbol == "easynet_publication_build_disable_ability_impl_invocation":
            return PUBLICATION_DISABLE_IMPL_INVOCATION
        if symbol == "easynet_publication_project_disable_ability_impl_result":
            return PUBLICATION_DISABLE_IMPL_PROJECTION
        if symbol == "easynet_admin_build_agent_list_invocation":
            return ADMIN_AGENT_LIST_INVOCATION
        if symbol == "easynet_admin_build_agent_start_invocation":
            return ADMIN_AGENT_START_INVOCATION
        if symbol == "easynet_admin_build_agent_stop_invocation":
            return ADMIN_AGENT_STOP_INVOCATION
        if symbol == "easynet_admin_build_agent_refresh_invocation":
            return ADMIN_AGENT_REFRESH_INVOCATION
        if symbol == "easynet_admin_build_session_list_invocation":
            return ADMIN_SESSION_LIST_INVOCATION
        if symbol in {
            "easynet_mission_build_run_eal_invocation",
            "easynet_mission_build_run_file_invocation",
        }:
            return MISSION_RUN_INVOCATION
        if symbol == "easynet_mission_build_track_invocation":
            return MISSION_TRACK_INVOCATION
        if symbol == "easynet_mission_build_cancel_invocation":
            return MISSION_CANCEL_INVOCATION
        if symbol == "easynet_surface_build_list_pages_invocation":
            return SURFACE_LIST_PAGES_INVOCATION
        if symbol == "easynet_surface_build_create_page_invocation":
            return SURFACE_CREATE_PAGE_INVOCATION
        if symbol == "easynet_surface_build_delete_page_invocation":
            return SURFACE_DELETE_PAGE_INVOCATION
        if symbol == "easynet_surface_build_manifest_invocation":
            return SURFACE_MANIFEST_INVOCATION
        if symbol == "easynet_surface_build_health_invocation":
            return SURFACE_HEALTH_INVOCATION
        if symbol == "easynet_compatibility_build_list_models_invocation":
            return COMPAT_LIST_MODELS_INVOCATION
        if symbol == "easynet_compatibility_build_chat_completion_invocation":
            return COMPAT_CHAT_COMPLETION_INVOCATION
        if symbol == "easynet_compatibility_build_stream_chat_completion_invocation":
            return COMPAT_STREAM_CHAT_COMPLETION_INVOCATION
        if symbol == "easynet_compatibility_build_file_upload_invocation":
            return COMPAT_FILE_UPLOAD_INVOCATION
        if symbol == "easynet_compatibility_build_file_retrieve_invocation":
            return COMPAT_FILE_RETRIEVE_INVOCATION
        if symbol == "easynet_compatibility_build_file_delete_invocation":
            return COMPAT_FILE_DELETE_INVOCATION
        if symbol == "easynet_wrappers_build_file_transfer_invocation":
            return WRAPPER_FILE_TRANSFER_INVOCATION
        if symbol == "easynet_wrappers_build_terminal_session_invocation":
            return WRAPPER_TERMINAL_INVOCATION
        if symbol == "easynet_wrappers_build_remote_desktop_session_invocation":
            return WRAPPER_REMOTE_DESKTOP_INVOCATION
        if symbol == "easynet_wrappers_build_browser_session_invocation":
            return WRAPPER_BROWSER_INVOCATION
        if symbol == "easynet_wrappers_build_media_session_invocation":
            return WRAPPER_MEDIA_INVOCATION
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
            if isinstance(request, dict) and "agents_scanned" in request:
                return ADMIN_REFRESH_RESULT_PROJECTION
            if isinstance(request, dict) and "agent_ura" in request:
                return ADMIN_START_RESULT_PROJECTION
            return ADMIN_STOP_RESULT_PROJECTION
        if symbol == "easynet_admin_project_device_session_page":
            return ADMIN_DEVICE_SESSION_PAGE_PROJECTION
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
        if symbol == "easynet_surface_project_health":
            return SURFACE_HEALTH_PROJECTION
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

PUBLICATION_PLUGIN_INSTALL_PROJECTION = (
    b'{"profile":"publication","kind":"plugin_install",'
    b'"source":"file:///tmp/plugin","install_id":"test.plugin@0.1.0",'
    b'"status":"installed","metadata":{"profile":"publication",'
    b'"package_id":"test.plugin","version":"0.1.0",'
    b'"hash":"sha256:abc","request_metadata":{}}}'
)

EVENTS_DIRECTORY_SUBSCRIPTION_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/'
    b'device.dev-a.federation.subscribe_directory_v2@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"stream":"directory","resume_cursor":{"stream":"directory","sequence":8}},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"events",'
    b'"system_ability":"federation.subscribe_directory_v2",'
    b'"carrier_owner":"daemon_sdk"}}'
)

EVENTS_SESSION_SUBSCRIPTION_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/'
    b'device.dev-a.session.attach@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"session_id":"run-1","since_seq":4},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"events",'
    b'"system_ability":"session.attach",'
    b'"carrier_owner":"daemon_sdk"}}'
)

PUBLICATION_DEPLOY_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/'
    b'device.dev-a.ability.deploy@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"resource_ref":{'
    b'"resource_ura":"easynet:///r/example/resource/device.dev-a/fs/tmp/package",'
    b'"owner_ura":"easynet:///r/example/device/dev-a",'
    b'"namespace":"fs","display_path":"tmp/package","capability":"read",'
    b'"expires_unix_ms":4102444800000,"revision":"fs-local-mapping-v1"},'
    b'"node_id":"local"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"publication","system_ability":"ability.deploy",'
    b'"carrier_owner":"daemon_sdk"}}'
)

PUBLICATION_DEPLOY_RESULT_PROJECTION = (
    b'{"profile":"publication","kind":"ability_deploy_result",'
    b'"public_name":"weather","namespace":"er",'
    b'"ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
    b'"node_id":"local","install_id":"install-1","state":"enabled",'
    b'"mutated_by":"","bundle":"","metadata":{}}'
)

PUBLICATION_LIST_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/'
    b'device.dev-a.meta.list_abilities@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"publication","system_ability":"meta.list_abilities",'
    b'"carrier_owner":"daemon_sdk"}}'
)

PUBLICATION_ABILITY_PAGE = (
    b'{"profile":"publication","kind":"published_ability_page",'
    b'"item_kind":"published_ability","items":[{"descriptor":{'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.fs.read@1.0.0",'
    b'"descriptor_version":"1.0.0","schema_hash":"sha256:abc",'
    b'"owner_ura":"easynet:///r/example/device/dev-a",'
    b'"ability_ura":"easynet:///r/example/ability/device.dev-a.fs.read"},'
    b'"implementation":{},"metadata":{"source_ability":"meta.list_abilities"}}],'
    b'"next_cursor":null,"limit":50,"source":"read_model",'
    b'"metadata":{"profile":"publication","source_ability":"meta.list_abilities"}}'
)

PUBLICATION_SHOW_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/'
    b'device.dev-a.meta.list_abilities@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"subject_ura":"easynet:///r/example/ability/device.dev-a.er.weather"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"publication","system_ability":"meta.list_abilities",'
    b'"carrier_owner":"daemon_sdk"}}'
)

PUBLICATION_ABILITY_RECORD = (
    b'{"descriptor":{"descriptor_ref":"easynet:///r/example/ability/'
    b'device.dev-a.er.weather@1.0.0","descriptor_version":"1.0.0",'
    b'"ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
    b'"owner_ura":"easynet:///r/example/device/dev-a"},'
    b'"implementation":{},"metadata":{"profile":"publication",'
    b'"source_ability":"meta.list_abilities"}}'
)

PUBLICATION_UNPUBLISH_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/'
    b'device.dev-a.ability.unpublish@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"publication","system_ability":"ability.unpublish",'
    b'"carrier_owner":"daemon_sdk"}}'
)

PUBLICATION_UNPUBLISH_RESULT_PROJECTION = (
    b'{"profile":"publication","kind":"ability_unpublished",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",'
    b'"owner_ura":"easynet:///r/example/device/dev-a",'
    b'"status":"unpublished","metadata":{"profile":"publication",'
    b'"source_ability":"ability.unpublish",'
    b'"content_hash":"sha256:abc"}}'
)

PUBLICATION_ENABLE_IMPL_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/'
    b'device.dev-a.ability.impl.enable@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"impl_id":"impl-1",'
    b'"ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"publication","system_ability":"ability.impl.enable",'
    b'"carrier_owner":"daemon_sdk"}}'
)

PUBLICATION_DISABLE_IMPL_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/'
    b'device.dev-a.ability.impl.disable@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"impl_id":"impl-1",'
    b'"ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"publication","system_ability":"ability.impl.disable",'
    b'"carrier_owner":"daemon_sdk"}}'
)

PUBLICATION_ENABLE_IMPL_PROJECTION = (
    b'{"profile":"publication","kind":"ability_impl_enabled",'
    b'"owner_ura":"easynet:///r/example/device/dev-a",'
    b'"status":"enabled","metadata":{"profile":"publication",'
    b'"source_ability":"ability.impl.enable",'
    b'"ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
    b'"impl_id":"impl-1"}}'
)

PUBLICATION_DISABLE_IMPL_PROJECTION = (
    b'{"profile":"publication","kind":"ability_impl_disabled",'
    b'"owner_ura":"easynet:///r/example/device/dev-a",'
    b'"status":"disabled","metadata":{"profile":"publication",'
    b'"source_ability":"ability.impl.disable",'
    b'"ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather",'
    b'"impl_id":"impl-1"}}'
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
    b'{"receipt_ura":"easynet:///r/example/receipt/receipt-1",'
    b'"receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",'
    b'"verified":false,'
    b'"causal_context":{"form":"scalar",'
    b'"receipt_ura":"easynet:///r/example/receipt/receipt-1",'
    b'"receipt_hash_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},'
    b'"invocation_id":"inv-example-1","metadata":{}}'
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

RECEIPT_LIST_HISTORY_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/'
    b'device.dev-a.invocation.history.list@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"limit":5},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"receipt",'
    b'"system_ability":"invocation.history.list",'
    b'"carrier_owner":"daemon_sdk","timeout_ms":2500}}'
)

RECEIPT_GET_HISTORY_INVOCATION = (
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
    b'"carrier_owner":"daemon_sdk","timeout_ms":2500}}'
)

RECEIPT_TRACE_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/'
    b'device.dev-a.invocation.trace.get@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"key":{"trace_id":"trace-1"}},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"receipt",'
    b'"system_ability":"invocation.trace.get",'
    b'"carrier_owner":"daemon_sdk","timeout_ms":2500}}'
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

IDENTITY_REGISTER_SIGNING_KEY_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/'
    b'device.dev-a.identity.register_pubkey@1.0.0",'
    b'"subject_ura":"easynet:///r/example/user/alice",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"agent_ura":"easynet:///r/example/user/alice",'
    b'"public_key_b64":"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=",'
    b'"role":"user"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"directory_identity",'
    b'"system_ability":"identity.register_pubkey",'
    b'"carrier_owner":"daemon_sdk"}}'
)

IDENTITY_LIST_SIGNING_KEYS_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/'
    b'device.dev-a.identity.list_user_pubkeys@1.0.0",'
    b'"subject_ura":"easynet:///r/example/user/alice",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"agent_ura":"easynet:///r/example/user/alice"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"directory_identity",'
    b'"system_ability":"identity.list_user_pubkeys",'
    b'"carrier_owner":"daemon_sdk"}}'
)

IDENTITY_REVOKE_SIGNING_KEY_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/'
    b'device.dev-a.identity.revoke_user_pubkey@1.0.0",'
    b'"subject_ura":"easynet:///r/example/user/alice",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"agent_ura":"easynet:///r/example/user/alice",'
    b'"public_key_b64":"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE="},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"directory_identity",'
    b'"system_ability":"identity.revoke_user_pubkey",'
    b'"carrier_owner":"daemon_sdk"}}'
)

IDENTITY_SIGNING_KEY_RECORD = (
    b'{"profile":"directory_identity","key_id":"alice-key-1",'
    b'"owner_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"algorithm":"ed25519",'
    b'"public_key_base64":"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=",'
    b'"state":"active","usage":["invocation.sign"],'
    b'"created_unix_ms":0,"revoked_unix_ms":0,'
    b'"metadata":{"source":"identity.register_pubkey"}}'
)

IDENTITY_SIGNING_KEY_PAGE = (
    b'{"profile":"directory_identity","items":['
    b'{"profile":"directory_identity","key_id":"ed25519:derived",'
    b'"owner_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"algorithm":"ed25519",'
    b'"public_key_base64":"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=",'
    b'"state":"active","usage":["invocation.sign"],'
    b'"created_unix_ms":1783100000123,"revoked_unix_ms":0,'
    b'"metadata":{"source":"identity.list_user_pubkeys"}}],'
    b'"next_cursor":null,"limit":50,'
    b'"metadata":{"source":"identity.list_user_pubkeys","rotation_epoch":3}}'
)

IDENTITY_SIGNING_KEY_REVOKE = (
    b'{"profile":"directory_identity","key_id":"alice-key-1",'
    b'"revoked":true,"state":"revoked",'
    b'"metadata":{"source":"identity.revoke_user_pubkey","removed":true}}'
)

IDENTITY_SIGNER_HANDLE = (
    b'{"profile":"directory_identity","signer_id":"signer-ed25519:derived",'
    b'"owner_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"key_id":"ed25519:derived","algorithm":"ed25519",'
    b'"policy":{"mode":"local_daemon_signing","usage":"invocation.sign",'
    b'"signer_id":"signer-ed25519:derived"},'
    b'"metadata":{"source":"identity.list_user_pubkeys"}}'
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

MISSION_RUN_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.mission.run@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"source":"mission demo"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"mission","system_ability":"mission.run",'
    b'"carrier_owner":"daemon_sdk"}}'
)

MISSION_TRACK_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.mission.track@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"run_id":"mission-1"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"mission","system_ability":"mission.track",'
    b'"carrier_owner":"daemon_sdk"}}'
)

MISSION_CANCEL_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.mission.cancel@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"run_id":"mission-1"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"mission","system_ability":"mission.cancel",'
    b'"carrier_owner":"daemon_sdk"}}'
)

ADMIN_AGENT_LIST_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.agent.list@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"admin_gateway","system_ability":"agent.list",'
    b'"carrier_owner":"daemon_sdk"}}'
)

ADMIN_AGENT_START_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.agent.start@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"name":"codex","agent_type":"codex"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"admin_gateway","system_ability":"agent.start",'
    b'"carrier_owner":"daemon_sdk"}}'
)

ADMIN_AGENT_STOP_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.agent.stop@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"name":"codex"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"admin_gateway","system_ability":"agent.stop",'
    b'"carrier_owner":"daemon_sdk"}}'
)

ADMIN_AGENT_REFRESH_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.agent.refresh@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"name":"codex"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"admin_gateway","system_ability":"agent.refresh",'
    b'"carrier_owner":"daemon_sdk"}}'
)

ADMIN_SESSION_LIST_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.session.list@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"include_terminated":false},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"admin_gateway","system_ability":"session.list",'
    b'"carrier_owner":"daemon_sdk"}}'
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
    b'{"profile":"admin_gateway","gateway_id":"local-daemon",'
    b'"state":"ready","ready":true,"process_live":true,'
    b'"control_ready":true,"runtime_ready":true,"directory_ready":true,'
    b'"trust_ready":true,"public_listener_ready":false,'
    b'"listeners":[],"identity":null,'
    b'"metadata":{"profile":"admin_gateway",'
    b'"source":"daemon_lifecycle_status"}}'
)

ADMIN_AGENT_PAGE_PROJECTION = (
    b'{"profile":"admin_gateway","kind":"agent_records","items":[],'
    b'"state":"ok","next_cursor":null,"limit":50,"metadata":{}}'
)

ADMIN_DEVICE_SESSION_PAGE_PROJECTION = (
    b'{"profile":"admin_gateway","kind":"device_sessions","state":"ok",'
    b'"items":[{"profile":"admin_gateway","kind":"device_session",'
    b'"session_id":"dev-session-1","device_ura":"easynet:///r/example/device/dev-a",'
    b'"hub_ura":"easynet:///r/example/hub","state":"active",'
    b'"session_kind":"daemon_session","created_unix_ms":1767225600000,'
    b'"expires_unix_ms":0,"metadata":{"profile":"admin_gateway",'
    b'"source":"session.list","agent":"codex"}}],'
    b'"next_cursor":null,"metadata":{"profile":"admin_gateway",'
    b'"source":"session.list","count":1}}'
)

ADMIN_START_RESULT_PROJECTION = (
    b'{"profile":"admin_gateway","kind":"agent_lifecycle_result",'
    b'"operation":"agent.start","state":"ok",'
    b'"agent_ura":"easynet:///r/example/agent/alice.codex",'
    b'"ack":true,"runtime_not_ready":false,'
    b'"runtime_catalog_not_ready":false,"metadata":{}}'
)

ADMIN_STOP_RESULT_PROJECTION = (
    b'{"profile":"admin_gateway","kind":"agent_lifecycle_result",'
    b'"operation":"agent.stop","state":"ok","agent_ura":null,'
    b'"ack":true,"runtime_not_ready":false,'
    b'"runtime_catalog_not_ready":false,"metadata":{}}'
)

ADMIN_REFRESH_RESULT_PROJECTION = (
    b'{"profile":"admin_gateway","kind":"agent_lifecycle_result",'
    b'"operation":"agent.refresh","state":"ok","agent_ura":null,'
    b'"ack":null,"runtime_not_ready":false,'
    b'"runtime_catalog_not_ready":false,"metadata":{}}'
)

SURFACE_LIST_PAGES_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/agent/alice.pages",'
    b'"descriptor_ref":"easynet:///r/example/ability/alice.pages.pages.list@1.0.0",'
    b'"subject_ura":"easynet:///r/example/agent/alice.pages",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"surface","system_ability":"pages.list",'
    b'"carrier_owner":"daemon_sdk"}}'
)

SURFACE_CREATE_PAGE_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/agent/alice.pages",'
    b'"descriptor_ref":"easynet:///r/example/ability/alice.pages.pages.publish@1.0.0",'
    b'"subject_ura":"easynet:///r/example/agent/alice.pages",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"project_id":"docs","folder":"/tmp/easynet-pages-docs",'
    b'"visibility":"public"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"surface","system_ability":"pages.publish",'
    b'"carrier_owner":"daemon_sdk"}}'
)

SURFACE_DELETE_PAGE_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/agent/alice.pages",'
    b'"descriptor_ref":"easynet:///r/example/ability/alice.pages.pages.unpublish@1.0.0",'
    b'"subject_ura":"easynet:///r/example/agent/alice.pages",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"project_id":"docs"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"surface","system_ability":"pages.unpublish",'
    b'"carrier_owner":"daemon_sdk"}}'
)

SURFACE_MANIFEST_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/agent/alice.pages",'
    b'"descriptor_ref":"easynet:///r/example/ability/alice.pages.pages.get@1.0.0",'
    b'"subject_ura":"easynet:///r/example/agent/alice.pages",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"project_id":"docs"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"surface","system_ability":"pages.get",'
    b'"carrier_owner":"daemon_sdk"}}'
)

SURFACE_HEALTH_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/agent/alice.pages",'
    b'"descriptor_ref":"easynet:///r/example/ability/alice.pages.pages.health@1.0.0",'
    b'"subject_ura":"easynet:///r/example/agent/alice.pages",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"project_id":"docs"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"surface","system_ability":"pages.health",'
    b'"carrier_owner":"daemon_sdk"}}'
)

SURFACE_PAGE_RECORD_PROJECTION = (
    b'{"profile":"surface","kind":"page_record","page_id":"docs",'
    b'"owner_ura":"easynet:///r/example/agent/alice.pages",'
    b'"surface_ref":"easynet:///r/example/resource/alice.docs",'
    b'"public_ref":"https://example/web/alice/docs/","status":"published",'
    b'"metadata":{"profile":"surface","source_ability":"pages.get",'
    b'"project_id":"docs"}}'
)

SURFACE_PAGE_PAGE_PROJECTION = (
    b'{"profile":"surface","kind":"surface_page_page","item_kind":"page_record",'
    b'"items":[{"profile":"surface","kind":"page_record","page_id":"docs",'
    b'"owner_ura":"easynet:///r/example/agent/alice.pages",'
    b'"surface_ref":"easynet:///r/example/resource/alice.docs",'
    b'"public_ref":"https://example/web/alice/docs/","status":"published",'
    b'"metadata":{"profile":"surface","source_ability":"pages.get",'
    b'"project_id":"docs"}}],"next_cursor":null,"limit":50,'
    b'"source":"pages_read_model","metadata":{"profile":"surface",'
    b'"source_ability":"pages.list","total_available":1}}'
)

SURFACE_MANIFEST_PROJECTION = (
    b'{"profile":"surface","kind":"surface_manifest","page_id":"docs",'
    b'"owner_ura":"easynet:///r/example/agent/alice.pages",'
    b'"surface_ref":"easynet:///r/example/resource/alice.docs",'
    b'"public_ref":"https://example/web/alice/docs/",'
    b'"page":{"profile":"surface","kind":"page_record","page_id":"docs",'
    b'"owner_ura":"easynet:///r/example/agent/alice.pages",'
    b'"surface_ref":"easynet:///r/example/resource/alice.docs",'
    b'"public_ref":"https://example/web/alice/docs/","status":"published",'
    b'"metadata":{"profile":"surface","source_ability":"pages.get",'
    b'"project_id":"docs"}},'
    b'"entrypoint":{"kind":"public_page_ref",'
    b'"href":"https://example/web/alice/docs/"},'
    b'"metadata":{"profile":"surface","source_ability":"pages.get"}}'
)

SURFACE_PUBLIC_REF_PROJECTION = (
    b'{"profile":"surface","kind":"public_page_ref","page_id":"docs",'
    b'"owner_ura":"easynet:///r/example/agent/alice.pages",'
    b'"surface_ref":"easynet:///r/example/resource/alice.docs",'
    b'"public_ref":"https://example/web/alice/docs/",'
    b'"route_kind":"hub_web","metadata":{"profile":"surface"}}'
)

SURFACE_MUTATION_PROJECTION = (
    b'{"profile":"surface","kind":"surface_mutation_result","operation":"delete",'
    b'"page_id":"docs","removed":true,"state":"deleted",'
    b'"metadata":{"profile":"surface","source_ability":"pages.unpublish"}}'
)

SURFACE_HEALTH_PROJECTION = (
    b'{"profile":"surface","kind":"surface_health","state":"ready","ready":true,'
    b'"owner_ura":"easynet:///r/example/agent/alice.pages",'
    b'"surface_ref":"easynet:///r/example/resource/alice.docs",'
    b'"descriptor_ref":"easynet:///r/example/ability/alice.pages.pages.health@1.0.0",'
    b'"descriptor_version":"1.0.0","page_count":1,'
    b'"checks":[{"name":"manifest","state":"ready","ready":true,'
    b'"message":null,"latency_ms":3,"metadata":{"source":"pages.get"}}],'
    b'"metadata":{"profile":"surface","source_ability":"pages.health",'
    b'"rendering_owner":"backend"}}'
)

COMPAT_LIST_MODELS_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.openai.list_models@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"auth_token":"tok_example"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"compatibility","system_ability":"openai.list_models",'
    b'"carrier_owner":"daemon_sdk"}}'
)

COMPAT_CHAT_COMPLETION_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.openai.chat_completions@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"request":{"model":"easynet:///r/example/ability/alice.codex.chat",'
    b'"messages":[{"role":"user","content":"reply with: ok"}]}},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"compatibility",'
    b'"system_ability":"openai.chat_completions",'
    b'"carrier_owner":"daemon_sdk"}}'
)

COMPAT_STREAM_CHAT_COMPLETION_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.openai.chat_completions@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"request":{"model":"easynet:///r/example/ability/alice.codex.chat",'
    b'"messages":[{"role":"user","content":"reply with: ok"}],'
    b'"stream":true}},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"compatibility",'
    b'"system_ability":"openai.chat_completions",'
    b'"carrier_owner":"daemon_sdk"}}'
)

COMPAT_FILE_UPLOAD_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.openai.files.upload@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"file_ref":"easynet:///r/example/resource/alice.files/prompt.jsonl",'
    b'"purpose":"batch"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"compatibility",'
    b'"system_ability":"openai.files.upload",'
    b'"carrier_owner":"daemon_sdk"}}'
)

COMPAT_FILE_RETRIEVE_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.openai.files.retrieve@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"file_id":"file-easynet-docs-1"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"compatibility",'
    b'"system_ability":"openai.files.retrieve",'
    b'"carrier_owner":"daemon_sdk"}}'
)

COMPAT_FILE_DELETE_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.openai.files.delete@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"file_id":"file-easynet-docs-1","deleted":true},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"compatibility",'
    b'"system_ability":"openai.files.delete",'
    b'"carrier_owner":"daemon_sdk"}}'
)

COMPAT_MODEL_PAGE_PROJECTION = (
    b'{"profile":"compatibility","kind":"model_page","object":"list",'
    b'"data":[{"profile":"compatibility","kind":"model",'
    b'"id":"easynet:///r/example/ability/alice.codex.chat",'
    b'"object":"model","created":0,"owned_by":"easynet",'
    b'"ability_ref":"easynet:///r/example/ability/alice.codex.chat",'
    b'"metadata":{"profile":"compatibility","source":"openai.list_models"}}],'
    b'"next_cursor":null,'
    b'"metadata":{"profile":"compatibility","source":"openai.list_models",'
    b'"count":1}}'
)

COMPAT_CHAT_PROJECTION = (
    b'{"profile":"compatibility","kind":"chat_completion",'
    b'"id":"chatcmpl-example","object":"chat.completion","created":1,'
    b'"model":"easynet:///r/example/ability/alice.codex.chat",'
    b'"choices":[{"index":0,"message":{"role":"assistant","content":"ok"},'
    b'"finish_reason":"stop"}],'
    b'"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4},'
    b'"metadata":{"profile":"compatibility","source":"openai.chat_completions"}}'
)

COMPAT_STREAM_PROJECTION = (
    b'{"profile":"compatibility","kind":"chat_completion_stream",'
    b'"stream":true,"items":[{"profile":"compatibility",'
    b'"kind":"chat_completion_chunk","id":"chatcmpl-stream-example",'
    b'"object":"chat.completion.chunk","created":1,'
    b'"model":"easynet:///r/example/ability/alice.codex.chat",'
    b'"choices":[{"index":0,"delta":{"content":"ok"},'
    b'"finish_reason":null}],"usage":null,'
    b'"metadata":{"profile":"compatibility",'
    b'"source":"openai.chat_completions"}}],'
    b'"done_sentinel":"[DONE]",'
    b'"metadata":{"profile":"compatibility",'
    b'"source":"openai.chat_completions"}}'
)

COMPAT_FILE_PROJECTION = (
    b'{"profile":"compatibility","kind":"file","id":"file-easynet-docs-1",'
    b'"object":"file","bytes":19,"created_at":1783094400,'
    b'"filename":"prompt.jsonl","purpose":"batch","status":"processed",'
    b'"metadata":{"profile":"compatibility","source":"compatibility.file",'
    b'"file_ref":"easynet:///r/example/resource/alice.files/prompt.jsonl"}}'
)

COMPAT_FILE_DELETE_PROJECTION = (
    b'{"profile":"compatibility","kind":"file_delete_result",'
    b'"id":"file-easynet-docs-1","object":"file","deleted":true,'
    b'"metadata":{"profile":"compatibility","source":"openai.files.delete"}}'
)

WRAPPER_FILE_TRANSFER_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.wrapper.file.transfer@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"wrapper_kind":"file","operation":"transfer"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"wrappers","system_ability":"wrapper.file.transfer",'
    b'"carrier_owner":"daemon_sdk"}}'
)

WRAPPER_TERMINAL_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.wrapper.terminal.start@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"wrapper_kind":"terminal","session_id":"term-1"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"wrappers","system_ability":"wrapper.terminal.start",'
    b'"carrier_owner":"daemon_sdk"}}'
)

WRAPPER_REMOTE_DESKTOP_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.wrapper.remote_desktop.start@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"wrapper_kind":"remote_desktop","session_id":"rdp-1"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"wrappers",'
    b'"system_ability":"wrapper.remote_desktop.start",'
    b'"carrier_owner":"daemon_sdk"}}'
)

WRAPPER_BROWSER_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.wrapper.browser.start@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"wrapper_kind":"browser","session_id":"browser-1"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"wrappers","system_ability":"wrapper.browser.start",'
    b'"carrier_owner":"daemon_sdk"}}'
)

WRAPPER_MEDIA_INVOCATION = (
    b'{"caller_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"callee_ura":"easynet:///r/example/device/dev-a",'
    b'"descriptor_ref":"easynet:///r/example/ability/device.dev-a.wrapper.media.start@1.0.0",'
    b'"subject_ura":"easynet:///r/example/device/dev-a",'
    b'"nonce_base64":"AQIDBAUGBwgJCgsMDQ4PEA==",'
    b'"causal_context":{"form":"none"},'
    b'"args":{"wrapper_kind":"media","session_id":"media-1"},'
    b'"content_type":"application/json",'
    b'"metadata":{"profile":"wrappers","system_ability":"wrapper.media.start",'
    b'"carrier_owner":"daemon_sdk"}}'
)

WRAPPER_FILE_PROJECTION = (
    b'{"profile":"wrappers","kind":"file_record",'
    b'"file_ref":"easynet:///r/example/resource/alice.files/report.txt",'
    b'"owner_ura":"easynet:///r/example/agent/alice.sdk",'
    b'"content_type":"text/plain","size_bytes":42,'
    b'"content_hash":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",'
    b'"metadata":{"profile":"wrappers","source":"wrappers.file_record"}}'
)

WRAPPER_TERMINAL_PROJECTION = (
    b'{"profile":"wrappers","kind":"terminal_session","session_id":"term-1",'
    b'"owner_ura":"easynet:///r/example/agent/alice.sdk","state":"ready",'
    b'"terminal_ref":"pty:1",'
    b'"metadata":{"profile":"wrappers","source":"wrappers.terminal_session"}}'
)

WRAPPER_REMOTE_DESKTOP_PROJECTION = (
    b'{"profile":"wrappers","kind":"remote_desktop_session","session_id":"rdp-1",'
    b'"owner_ura":"easynet:///r/example/agent/alice.sdk","state":"ready",'
    b'"display_ref":"display:1",'
    b'"metadata":{"profile":"wrappers","source":"wrappers.remote_desktop_session"}}'
)

WRAPPER_BROWSER_PROJECTION = (
    b'{"profile":"wrappers","kind":"browser_session","session_id":"browser-1",'
    b'"owner_ura":"easynet:///r/example/agent/alice.sdk","state":"ready",'
    b'"browser_ref":"browser:1",'
    b'"metadata":{"profile":"wrappers","source":"wrappers.browser_session"}}'
)

WRAPPER_MEDIA_PROJECTION = (
    b'{"profile":"wrappers","kind":"media_session","session_id":"media-1",'
    b'"owner_ura":"easynet:///r/example/agent/alice.sdk","state":"ready",'
    b'"media_kind":"voice","stream_ref":"stream:1",'
    b'"metadata":{"profile":"wrappers","source":"wrappers.media_session"}}'
)

CURRENT_ABI_PREPARED = b"""{
  "request_id": "req-current-1",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
  "descriptor_hash_hex": "aa",
  "schema_hash_hex": "bb",
  "canonical_hash_hex": "50d858e0985ecc7f60418aaf0cc5ab587f42c2570a884095a9e8ccacd0f6545c",
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
        self.assertTrue(features.symbols["identity_signing_key_lifecycle"])
        self.assertTrue(features.symbols["events_session_stream"])

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

    def test_identity_transport_builds_resource_ref_through_cabi_projector(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = IdentityClient(CABIIdentityTransport(lib, handle=7))

        ref = client.build_resource_ref(
            LocalResourceRefRequest(
                path="/tmp/package",
                capability="read",
            )
        )

        self.assertEqual(
            ref.resource_ura,
            "easynet:///r/example/resource/device.dev-a/fs/tmp/package",
        )
        self.assertEqual(ref.owner_ura, "easynet:///r/example/device/dev-a")
        self.assertEqual(ref.capability, "read")
        self.assertEqual(
            raw.profile_requests,
            [
                (
                    "easynet_publication_build_resource_ref",
                    7,
                    {"capability": "read", "path": "/tmp/package"},
                )
            ],
        )
        self.assertEqual(raw.buffers, {})

    def test_identity_transport_builds_signing_key_invocation_carriers(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = IdentityClient(CABIIdentityTransport(lib, handle=7))
        base = IdentityCarrierBase(
            caller_ura="easynet:///r/example/agent/alice.sdk",
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="easynet:///r/example/user/alice",
            descriptor_version="1.0.0",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context={"form": "none"},
            metadata={"request_id": "identity-1"},
        )

        register = client.build_register_signing_key_invocation(
            SigningKeyRegistrationRequest(
                owner_ura="easynet:///r/example/user/alice",
                key_id="alice-key-1",
                algorithm="ed25519",
                public_key_base64="AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=",
                usage=("invocation.sign",),
                base=base,
            )
        )
        listed = client.build_list_signing_keys_invocation(
            SigningKeyListRequest(
                owner_ura="easynet:///r/example/user/alice",
                base=base,
            )
        )
        revoked = client.build_revoke_signing_key_invocation(
            SigningKeyRevokeRequest(
                key_id="alice-key-1",
                reason="rotation",
                owner_ura="easynet:///r/example/user/alice",
                public_key_base64="AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=",
                base=base,
            )
        )

        self.assertEqual(
            register.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.identity.register_pubkey@1.0.0",
        )
        self.assertEqual(
            listed.metadata["system_ability"], "identity.list_user_pubkeys"
        )
        self.assertEqual(
            revoked.metadata["system_ability"], "identity.revoke_user_pubkey"
        )
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            [
                "easynet_identity_build_register_signing_key_invocation",
                "easynet_identity_build_list_signing_keys_invocation",
                "easynet_identity_build_revoke_signing_key_invocation",
            ],
        )
        self.assertEqual(raw.profile_requests[0][2]["role"], "user")
        self.assertEqual(
            raw.profile_requests[0][2]["caller_ura"],
            "easynet:///r/example/agent/alice.sdk",
        )
        self.assertEqual(
            raw.profile_requests[2][2]["public_key_base64"],
            "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=",
        )
        self.assertEqual(raw.buffers, {})

    def test_identity_transport_executes_signing_key_lifecycle(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = IdentityClient(CABIIdentityTransport(lib, handle=7))

        record = client.register_signing_key(
            SigningKeyRegistrationRequest(
                owner_ura="easynet:///r/example/agent/alice.sdk",
                key_id="alice-key-1",
                algorithm="ed25519",
                public_key_base64="AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=",
                usage=("invocation.sign",),
            )
        )
        page = client.list_signing_keys(
            SigningKeyListRequest(
                owner_ura="easynet:///r/example/agent/alice.sdk",
            )
        )
        revoked = client.revoke_signing_key(
            SigningKeyRevokeRequest(
                key_id="alice-key-1",
                reason="rotation",
                owner_ura="easynet:///r/example/agent/alice.sdk",
                public_key_base64="AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=",
            )
        )

        self.assertEqual(record.key_id, "alice-key-1")
        self.assertEqual(page.items[0].owner_ura, "easynet:///r/example/agent/alice.sdk")
        self.assertTrue(revoked.revoked)
        self.assertEqual(
            [item[0] for item in raw.profile_requests[-6:]],
            [
                "easynet_identity_build_register_signing_key_invocation",
                "easynet_identity_project_signing_key_record",
                "easynet_identity_build_list_signing_keys_invocation",
                "easynet_identity_project_signing_key_page",
                "easynet_identity_build_revoke_signing_key_invocation",
                "easynet_identity_project_signing_key_revoke_result",
            ],
        )
        self.assertEqual(
            [item[1]["metadata"]["system_ability"] for item in raw.runtime_requests],
            [
                "identity.register_pubkey",
                "identity.list_user_pubkeys",
                "identity.revoke_user_pubkey",
            ],
        )

        signer = client.signer(
            SignerRequest(
                owner_ura="easynet:///r/example/agent/alice.sdk",
                key_id="ed25519:derived",
                usage="invocation.sign",
                base=IdentityCarrierBase(
                    caller_ura="easynet:///r/example/agent/alice.sdk",
                    callee_ura="easynet:///r/example/device/dev-a",
                    subject_ura="easynet:///r/example/agent/alice.sdk",
                    descriptor_version="1.0.0",
                    nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                    causal_context={"form": "none"},
                ),
            )
        )

        self.assertEqual(signer.signer_id, "signer-ed25519:derived")
        self.assertEqual(signer.key_id, "ed25519:derived")
        self.assertEqual(signer.algorithm, "ed25519")
        self.assertEqual(
            [item[0] for item in raw.profile_requests[-2:]],
            [
                "easynet_identity_build_list_signing_keys_invocation",
                "easynet_identity_project_signer_handle",
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

    def test_daemon_transport_exposes_invocation_endpoint_without_status_parse(
        self,
    ) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        transport = CABIDaemonTransport(lib)
        control = DaemonControl(transport)

        handle = control.attach(
            AttachOptions(control_endpoint="unix:///tmp/control.sock")
        )

        self.assertEqual(handle.invocation_endpoint(), "unix:///tmp/daemon.sock")
        self.assertEqual(raw.daemon_invocation_endpoint_calls, [707])

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

    def test_receipt_history_facade_uses_cabi_carriers_and_runtime_invoke(
        self,
    ) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = ReceiptClient(CABIReceiptTransport(lib, handle=7))
        carrier = ReceiptCarrierBase(
            caller_ura="easynet:///r/example/agent/alice.sdk",
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="easynet:///r/example/device/dev-a",
            descriptor_version="1.0.0",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context={"form": "none"},
            timeout_ms=2500,
            metadata={"request_id": "history-1"},
        )

        list_draft = client.build_list_history_invocation(
            ReceiptHistoryReadRequest(carrier=carrier, arguments={"limit": 5})
        )
        page = client.list_history(
            ReceiptHistoryReadRequest(carrier=carrier, arguments={"limit": 5})
        )
        record = client.get_history(
            ReceiptHistoryReadRequest(
                carrier=carrier,
                arguments={"key": {"request_id": "inv-example-1"}},
            )
        )
        trace = client.get_trace(
            ReceiptHistoryReadRequest(
                carrier=carrier,
                arguments={"key": {"trace_id": "trace-1"}},
            )
        )

        self.assertEqual(
            list_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.invocation.history.list@1.0.0",
        )
        self.assertEqual(page["records"][0]["invocation_id"], "inv-example-1")
        self.assertEqual(record["record"]["state"], "completed")
        self.assertEqual(trace["trace_id"], "trace-1")
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            [
                "easynet_receipt_build_list_history_invocation",
                "easynet_receipt_build_list_history_invocation",
                "easynet_receipt_build_get_history_invocation",
                "easynet_receipt_build_trace_invocation",
            ],
        )
        self.assertEqual(raw.profile_requests[0][2]["timeout_ms"], 2500)
        self.assertEqual(raw.profile_requests[0][2]["metadata"]["request_id"], "history-1")
        self.assertEqual(
            [item[1]["metadata"]["system_ability"] for item in raw.runtime_requests],
            [
                "invocation.history.list",
                "invocation.history.get",
                "invocation.trace.get",
            ],
        )
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

    def test_directory_subscribe_opens_runtime_stream(self) -> None:
        raw = FakeRawCABI()
        raw.stream_events = []
        lib = CLILibrary(raw)
        client = DirectoryClient(CABIDirectoryTransport(lib, handle=7))

        subscription = client.subscribe_directory(
            DirectorySubscriptionRequest(
                DirectoryQueryBase(
                    caller_ura="easynet:///r/example/agent/alice.sdk",
                    callee_ura="easynet:///r/example/device/dev-a",
                    subject_ura="easynet:///r/example/device/dev-a",
                    descriptor_version="1.0.0",
                    nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                    causal_context={"form": "none"},
                ),
                resume_cursor=DirectorySubscriptionCursor("directory", 8),
            )
        )

        self.assertEqual(subscription.state, "Live")
        self.assertEqual(subscription.cursor.resume_token(), "directory:8")
        self.assertEqual(subscription.resume_token, "directory:8")
        self.assertEqual(subscription.events, ())
        self.assertEqual(subscription.metadata["stream_ability"], "federation.subscribe_directory_v2")
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            ["easynet_events_build_directory_subscription_invocation"],
        )
        self.assertEqual(raw.runtime_requests[0][0], "stream_open")
        self.assertEqual(
            raw.runtime_requests[0][1]["descriptor_ref"],
            "easynet:///r/example/ability/"
            "device.dev-a.federation.subscribe_directory_v2@1.0.0",
        )
        self.assertEqual(raw.stream_closes, [])

        subscription.close()

        self.assertEqual(subscription.state, "Closed")
        self.assertEqual(raw.stream_closes, [404])
        self.assertEqual(raw.stream_cancels, [])
        self.assertEqual(raw.buffers, {})

    def test_directory_client_close_closes_subscription_runtime_stream(self) -> None:
        raw = FakeRawCABI()
        raw.stream_events = []
        lib = CLILibrary(raw)
        client = DirectoryClient(CABIDirectoryTransport(lib, handle=7))

        client.subscribe_directory(
            DirectorySubscriptionRequest(
                DirectoryQueryBase(
                    caller_ura="easynet:///r/example/agent/alice.sdk",
                    callee_ura="easynet:///r/example/device/dev-a",
                    subject_ura="easynet:///r/example/device/dev-a",
                    descriptor_version="1.0.0",
                    nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                    causal_context={"form": "none"},
                ),
            )
        )
        client.close()

        self.assertEqual(raw.stream_closes, [404])
        self.assertEqual(raw.stream_cancels, [])

    def test_publication_list_uses_carrier_invoke_and_projection(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = PublicationClient(CABIPublicationTransport(lib, handle=7))

        page = client.list_abilities(
            PublishedAbilityQuery(
                caller_ura="easynet:///r/example/agent/alice.sdk",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura="easynet:///r/example/device/dev-a",
                descriptor_version="1.0.0",
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
            )
        )

        self.assertEqual(page.limit, 50)
        self.assertEqual(len(page.items), 1)
        self.assertEqual(
            page.items[0].descriptor["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.fs.read@1.0.0",
        )
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            [
                "easynet_publication_build_list_abilities_invocation",
                "easynet_publication_project_ability_page",
            ],
        )
        self.assertEqual(raw.runtime_requests[0][0], "invoke")
        self.assertEqual(
            raw.runtime_requests[0][1]["metadata"]["system_ability"],
            "meta.list_abilities",
        )
        self.assertIn("result", raw.profile_requests[1][2])
        self.assertEqual(raw.profile_requests[1][2]["limit"], 50)
        self.assertEqual(raw.buffers, {})

    def test_publication_show_uses_carrier_invoke_and_record_projection(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = PublicationClient(CABIPublicationTransport(lib, handle=7))

        ability = client.show_ability(
            PublishedAbilityShowRequest(
                caller_ura="easynet:///r/example/agent/alice.sdk",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura="easynet:///r/example/device/dev-a",
                descriptor_version="1.0.0",
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                descriptor_ref="easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
            )
        )

        self.assertEqual(
            ability.descriptor["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
        )
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            [
                "easynet_publication_build_show_ability_invocation",
                "easynet_publication_project_ability_record",
            ],
        )
        self.assertEqual(raw.runtime_requests[0][0], "invoke")
        self.assertEqual(
            raw.runtime_requests[0][1]["metadata"]["system_ability"],
            "meta.list_abilities",
        )
        self.assertEqual(
            raw.profile_requests[1][2]["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
        )
        self.assertEqual(raw.buffers, {})

    def test_publication_deploy_uses_carrier_invoke(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = PublicationClient(CABIPublicationTransport(lib, handle=7))

        result = client.deploy_ability(
            AbilityDeployRequest(
                caller_ura="easynet:///r/example/agent/alice.sdk",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura="easynet:///r/example/device/dev-a",
                descriptor_version="1.0.0",
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                resource_ref=ResourceRef.from_json(RESOURCE_REF_PROJECTION),
                node_id="local",
            )
        )

        self.assertEqual(result.state, "enabled")
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            [
                "easynet_publication_build_deploy_invocation",
                "easynet_publication_project_deploy_result",
            ],
        )
        self.assertEqual(raw.runtime_requests[0][0], "invoke")
        self.assertEqual(
            raw.runtime_requests[0][1]["metadata"]["system_ability"],
            "ability.deploy",
        )
        self.assertEqual(raw.profile_requests[1][2]["state"], "enabled")
        self.assertEqual(raw.buffers, {})

    def test_publication_unpublish_uses_carrier_invoke_and_projection(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = PublicationClient(CABIPublicationTransport(lib, handle=7))

        client.unpublish_ability(
            UnpublishAbilityRequest(
                caller_ura="easynet:///r/example/agent/alice.sdk",
                callee_ura="easynet:///r/example/device/dev-a",
                subject_ura="easynet:///r/example/device/dev-a",
                descriptor_version="1.0.0",
                nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                causal_context={"form": "none"},
                ability_ura="easynet:///r/example/ability/device.dev-a.er.weather",
            )
        )

        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            [
                "easynet_publication_build_unpublish_invocation",
                "easynet_publication_project_unpublish_result",
            ],
        )
        self.assertEqual(raw.runtime_requests[0][0], "invoke")
        self.assertEqual(
            raw.runtime_requests[0][1]["metadata"]["system_ability"],
            "ability.unpublish",
        )
        self.assertEqual(
            raw.profile_requests[1][2]["descriptor_version"],
            "1.0.0",
        )
        self.assertEqual(
            raw.profile_requests[1][2]["result"]["content_hash"],
            "sha256:abc",
        )
        self.assertEqual(raw.buffers, {})

    def test_publication_install_plugin_uses_cabi_lifecycle_contract(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = PublicationClient(CABIPublicationTransport(lib, handle=7))

        result = client.install_plugin("file:///tmp/plugin")

        self.assertEqual(result.kind, "plugin_install")
        self.assertEqual(result.install_id, "test.plugin@0.1.0")
        self.assertEqual(result.status, "installed")
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            ["easynet_publication_install_plugin"],
        )
        self.assertEqual(raw.profile_requests[0][2]["source"], "file:///tmp/plugin")
        self.assertEqual(raw.runtime_requests, [])
        self.assertEqual(raw.buffers, {})

    def test_publication_impl_lifecycle_uses_cabi_runtime_contracts(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = PublicationClient(CABIPublicationTransport(lib, handle=7))

        request = AbilityImplLifecycleRequest(
            caller_ura="easynet:///r/example/agent/alice.sdk",
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="easynet:///r/example/device/dev-a",
            descriptor_version="1.0.0",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context={"form": "none"},
            ability_ura="easynet:///r/example/ability/device.dev-a.er.weather",
            impl_id="impl-1",
        )

        client.enable_ability_impl(request)
        client.disable_ability_impl(request)

        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            [
                "easynet_publication_build_enable_ability_impl_invocation",
                "easynet_publication_project_enable_ability_impl_result",
                "easynet_publication_build_disable_ability_impl_invocation",
                "easynet_publication_project_disable_ability_impl_result",
            ],
        )
        self.assertEqual(raw.runtime_requests[0][0], "invoke")
        self.assertEqual(
            raw.runtime_requests[0][1]["metadata"]["system_ability"],
            "ability.impl.enable",
        )
        self.assertEqual(
            raw.runtime_requests[1][1]["metadata"]["system_ability"],
            "ability.impl.disable",
        )
        self.assertEqual(raw.buffers, {})

    def test_publication_impl_lifecycle_requires_complete_runtime_carrier(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = PublicationClient(CABIPublicationTransport(lib, handle=7))

        impl = AbilityImplID(
            ability_ura="easynet:///r/example/ability/device.dev-a.er.weather",
            impl_id="impl-1",
        )

        with self.assertRaises(SDKError) as caught:
            client.enable_ability_impl(impl)

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(raw.runtime_requests, [])

    def test_events_subscribe_directory_opens_runtime_stream(self) -> None:
        raw = FakeRawCABI()
        raw.stream_events = []
        lib = CLILibrary(raw)
        client = EventClient(CABIEventTransport(lib, handle=7))

        stream = client.subscribe_directory(
            EventsDirectorySubscriptionRequest(
                EventsCarrierBase(
                    caller_ura="easynet:///r/example/agent/alice.sdk",
                    callee_ura="easynet:///r/example/device/dev-a",
                    subject_ura="easynet:///r/example/device/dev-a",
                    descriptor_version="1.0.0",
                    nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                    causal_context={"form": "none"},
                ),
                resume_cursor=EventCursor("directory", 8),
            )
        )

        self.assertEqual(stream.stream, "directory")
        self.assertEqual(stream.state, "Open")
        self.assertEqual(stream.stream_id, "404")
        self.assertEqual(stream.resume_token, "directory:8")
        self.assertEqual(stream.metadata["stream_ability"], "federation.subscribe_directory_v2")
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            ["easynet_events_build_directory_subscription_invocation"],
        )
        self.assertEqual(raw.runtime_requests[0][0], "stream_open")
        self.assertEqual(
            raw.runtime_requests[0][1]["descriptor_ref"],
            "easynet:///r/example/ability/"
            "device.dev-a.federation.subscribe_directory_v2@1.0.0",
        )
        self.assertEqual(
            raw.runtime_requests[0][1]["args"]["resume_cursor"],
            {"stream": "directory", "sequence": 8},
        )
        self.assertEqual(raw.stream_closes, [])

        stream.close()

        self.assertEqual(stream.state, "Closed")
        self.assertEqual(raw.stream_closes, [404])
        self.assertEqual(raw.stream_cancels, [])
        self.assertEqual(raw.buffers, {})

    def test_events_session_subscription_uses_session_attach_stream(self) -> None:
        raw = FakeRawCABI()
        raw.stream_events = []
        lib = CLILibrary(raw)
        client = EventClient(CABIEventTransport(lib, handle=7))
        base = EventsCarrierBase(
            caller_ura="easynet:///r/example/agent/alice.sdk",
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="easynet:///r/example/device/dev-a",
            descriptor_version="1.0.0",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context={"form": "none"},
        )

        draft = client.build_session_subscription_invocation(
            EventsSessionSubscriptionRequest(
                base,
                stream="session",
                session_id="run-1",
                resume_cursor=EventCursor("session", 4),
            )
        )
        stream = client.subscribe_sessions(
            EventsSessionSubscriptionRequest(
                base,
                stream="session",
                session_id="run-1",
                resume_cursor=EventCursor("session", 4),
            )
        )

        self.assertEqual(
            draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.session.attach@1.0.0",
        )
        self.assertEqual(stream.stream, "session")
        self.assertEqual(stream.state, "Open")
        self.assertEqual(stream.resume_token, "session:4")
        self.assertEqual(stream.metadata["stream_ability"], "session.attach")
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            [
                "easynet_events_build_session_subscription_invocation",
                "easynet_events_build_session_subscription_invocation",
            ],
        )
        self.assertEqual(raw.runtime_requests[0][0], "stream_open")
        self.assertEqual(
            raw.runtime_requests[0][1]["metadata"]["system_ability"],
            "session.attach",
        )
        self.assertEqual(raw.runtime_requests[0][1]["args"]["since_seq"], 4)

        client.close()

        self.assertEqual(raw.stream_closes, [404])

    def test_events_client_close_closes_directory_runtime_stream(self) -> None:
        raw = FakeRawCABI()
        raw.stream_events = []
        lib = CLILibrary(raw)
        client = EventClient(CABIEventTransport(lib, handle=7))

        client.subscribe_directory(
            EventsDirectorySubscriptionRequest(
                EventsCarrierBase(
                    caller_ura="easynet:///r/example/agent/alice.sdk",
                    callee_ura="easynet:///r/example/device/dev-a",
                    subject_ura="easynet:///r/example/device/dev-a",
                    descriptor_version="1.0.0",
                    nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
                    causal_context={"form": "none"},
                ),
            )
        )
        client.close()

        self.assertEqual(raw.stream_closes, [404])
        self.assertEqual(raw.stream_cancels, [])

    def test_admin_live_agent_methods_use_carrier_invoke_and_projection(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = AdminClient(CABIAdminTransport(lib, handle=7))
        base = AdminCarrierBase(
            caller_ura="easynet:///r/example/agent/alice.sdk",
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="easynet:///r/example/device/dev-a",
            descriptor_version="1.0.0",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context={"form": "none"},
        )

        agents = client.list_agents(AdminAgentListRequest(base))
        started = client.agent_start(
            AdminAgentStartRequest(base=base, name="codex", agent_type="codex")
        )
        stopped = client.agent_stop(AdminAgentStopRequest(base=base, name="codex"))
        refreshed = client.agent_refresh(
            AdminAgentRefreshRequest(base=base, name="codex")
        )

        self.assertEqual(agents.profile, "admin_gateway")
        self.assertEqual(started.operation, "agent.start")
        self.assertEqual(stopped.operation, "agent.stop")
        self.assertEqual(refreshed.operation, "agent.refresh")
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            [
                "easynet_admin_build_agent_list_invocation",
                "easynet_admin_project_agent_records",
                "easynet_admin_build_agent_start_invocation",
                "easynet_admin_project_agent_lifecycle_result",
                "easynet_admin_build_agent_stop_invocation",
                "easynet_admin_project_agent_lifecycle_result",
                "easynet_admin_build_agent_refresh_invocation",
                "easynet_admin_project_agent_lifecycle_result",
            ],
        )
        self.assertEqual(
            [item[1]["metadata"]["system_ability"] for item in raw.runtime_requests],
            ["agent.list", "agent.start", "agent.stop", "agent.refresh"],
        )
        self.assertEqual(
            raw.profile_requests[1][2]["agents"][0]["name"],
            "codex",
        )
        self.assertEqual(
            raw.profile_requests[3][2]["agent_ura"],
            "easynet:///r/example/agent/alice.codex",
        )
        self.assertTrue(raw.profile_requests[5][2]["ack"])
        self.assertEqual(raw.profile_requests[7][2]["agents_scanned"], 1)
        self.assertEqual(raw.buffers, {})

    def test_admin_list_device_sessions_uses_carrier_invoke_and_projection(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = AdminClient(CABIAdminTransport(lib, handle=7))
        base = AdminCarrierBase(
            caller_ura="easynet:///r/example/agent/alice.sdk",
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="easynet:///r/example/device/dev-a",
            descriptor_version="1.0.0",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context={"form": "none"},
        )

        page = client.list_device_sessions(
            AdminSessionListRequest(base=base, include_terminated=False)
        )

        self.assertEqual(page.profile, "admin_gateway")
        self.assertEqual(page.items[0].session_id, "dev-session-1")
        self.assertEqual(page.items[0].session_kind, "daemon_session")
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            [
                "easynet_admin_build_session_list_invocation",
                "easynet_admin_project_device_session_page",
            ],
        )
        self.assertEqual(
            raw.runtime_requests[0][1]["metadata"]["system_ability"], "session.list"
        )
        self.assertEqual(
            raw.profile_requests[1][2]["sessions"][0]["id"], "dev-session-1"
        )
        self.assertEqual(raw.buffers, {})

    def test_admin_trust_and_session_mutations_report_daemon_contract_boundary(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = AdminClient(CABIAdminTransport(lib, handle=7))
        base = AdminCarrierBase(
            caller_ura="easynet:///r/example/agent/alice.sdk",
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="easynet:///r/example/device/dev-a",
            descriptor_version="1.0.0",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context={"form": "none"},
        )
        hub = "easynet:///r/example/hub/main"
        device = "easynet:///r/example/device/dev-a"

        cases = [
            (
                "hub lifecycle",
                lambda: client.join_hub(AdminJoinHubRequest(base, hub, device)),
                "requires a daemon/ABI hub lifecycle contract",
                "gateway readiness projections cannot be projected",
            ),
            (
                "leave hub",
                lambda: client.leave_hub(AdminLeaveHubRequest(base, hub, "rotation")),
                "requires a daemon/ABI hub lifecycle contract",
                "gateway readiness projections cannot be projected",
            ),
            (
                "pairing preflight",
                lambda: client.pairing_preflight(
                    PairingPreflightRequest(base, hub, device, ("invoke",))
                ),
                "requires a daemon/ABI pairing and device-credential lifecycle contract",
                "cannot be projected into trust mutation semantics",
            ),
            (
                "pairing create",
                lambda: client.create_pairing(
                    CreatePairingRequest(base, hub, device, 1_893_456_000_000)
                ),
                "requires a daemon/ABI pairing and device-credential lifecycle contract",
                "cannot be projected into trust mutation semantics",
            ),
            (
                "pairing validate",
                lambda: client.validate_pairing(
                    ValidatePairingRequest(base, "pair-token-value", device)
                ),
                "requires a daemon/ABI pairing and device-credential lifecycle contract",
                "cannot be projected into trust mutation semantics",
            ),
            (
                "credential verify",
                lambda: client.verify_device_credential(
                    VerifyDeviceCredentialRequest(base, "cred-dev-a", device, hub)
                ),
                "requires a daemon/ABI pairing and device-credential lifecycle contract",
                "cannot be projected into trust mutation semantics",
            ),
            (
                "device revoke",
                lambda: client.revoke_device(
                    RevokeDeviceRequest(base, device, "rotation")
                ),
                "requires a daemon/ABI pairing and device-credential lifecycle contract",
                "cannot be projected into trust mutation semantics",
            ),
            (
                "session create",
                lambda: client.create_device_session(
                    CreateDeviceSessionRequest(base, device, hub, "remote_desktop")
                ),
                "requires a daemon/ABI device-session lifecycle contract",
                "session.list read-model rows cannot be projected",
            ),
            (
                "session delete",
                lambda: client.delete_device_session(
                    DeleteDeviceSessionRequest(base, "dev-session-1", "done")
                ),
                "requires a daemon/ABI device-session lifecycle contract",
                "session.list read-model rows cannot be projected",
            ),
        ]

        for name, operation, expected, detail in cases:
            with self.subTest(name=name):
                with self.assertRaises(SDKError) as caught:
                    operation()
                self.assertTrue(is_code(caught.exception, ErrorCode.NOT_IMPLEMENTED))
                self.assertEqual(caught.exception.stage, "cabi")
                self.assertIn(expected, caught.exception.message)
                self.assertIn(detail, caught.exception.message)

        self.assertEqual(raw.profile_requests, [])
        self.assertEqual(raw.runtime_requests, [])
        self.assertEqual(raw.buffers, {})

    def test_admin_gateway_status_uses_daemon_lifecycle_status_projection(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = AdminClient(
            CABIAdminTransport(lib, handle=7, daemon_handle=707)
        )

        status = client.gateway_status(
            AdminGatewayStatusRequest(require_public_listener=False)
        )

        self.assertEqual(status.profile, "admin_gateway")
        self.assertTrue(status.ready)
        self.assertEqual(status.gateway_id, "local-daemon")
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            ["easynet_admin_project_gateway_status"],
        )
        self.assertEqual(raw.profile_requests[0][2]["runtime_status"], "Running")
        self.assertEqual(raw.profile_requests[0][2]["daemon"]["pid"], 42)
        self.assertEqual(
            raw.profile_requests[0][2]["require_public_listener"], False
        )

    def test_open_cabi_admin_transport_attaches_and_detaches_daemon_handle(self) -> None:
        raw = FakeRawCABI()
        with patch("easynet_sdk._cabi.CLILibrary.load", return_value=CLILibrary(raw)):
            transport = open_cabi_admin_transport(control_path="/tmp/control.json")
            self.assertIsInstance(transport, CABIAdminTransport)
            self.assertEqual(transport.daemon_handle, 707)
            self.assertEqual(
                raw.daemon_attaches,
                [{"control_path": "/tmp/control.json"}],
            )
            transport.close()

        self.assertEqual(raw.shutdown_handles, [42])
        self.assertEqual(raw.daemon_detaches, [707])

    def test_surface_live_methods_use_carrier_invoke_and_projection(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = SurfaceClient(CABISurfaceTransport(lib, handle=7))
        base = SurfaceCarrierBase(
            caller_ura="easynet:///r/example/agent/alice.sdk",
            callee_ura="easynet:///r/example/agent/alice.pages",
            subject_ura="easynet:///r/example/agent/alice.pages",
            descriptor_version="1.0.0",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context={"form": "none"},
        )

        pages = client.list_pages(SurfaceListPagesRequest(base=base, limit=50))
        created = client.create_page(
            SurfaceCreatePageRequest(
                base=base,
                project_id="docs",
                folder="/tmp/easynet-pages-docs",
                visibility="public",
            )
        )
        manifest = client.surface_manifest(
            SurfaceManifestRequest(base=base, project_id="docs")
        )
        public_ref = client.public_page_ref(created)
        health = client.surface_health(SurfaceHealthRequest(base=base, project_id="docs"))
        deleted = client.delete_page(
            SurfaceDeletePageRequest(base=base, project_id="docs")
        )

        self.assertEqual(pages.items[0].page_id, "docs")
        self.assertEqual(created.surface_ref, "easynet:///r/example/resource/alice.docs")
        self.assertEqual(manifest.page.page_id, "docs")
        self.assertEqual(public_ref.route_kind, "hub_web")
        self.assertTrue(health.ready)
        self.assertEqual(health.checks[0].name, "manifest")
        self.assertEqual(deleted.state, "deleted")
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            [
                "easynet_surface_build_list_pages_invocation",
                "easynet_surface_project_page_page",
                "easynet_surface_build_create_page_invocation",
                "easynet_surface_project_page_record",
                "easynet_surface_build_manifest_invocation",
                "easynet_surface_project_manifest",
                "easynet_surface_project_public_page_ref",
                "easynet_surface_build_health_invocation",
                "easynet_surface_project_health",
                "easynet_surface_build_delete_page_invocation",
                "easynet_surface_project_mutation_result",
            ],
        )
        self.assertEqual(
            [item[1]["metadata"]["system_ability"] for item in raw.runtime_requests],
            ["pages.list", "pages.publish", "pages.get", "pages.health", "pages.unpublish"],
        )
        self.assertEqual(raw.profile_requests[1][2]["limit"], 50)
        self.assertEqual(
            raw.profile_requests[1][2]["result"]["projects"][0]["page_id"],
            "docs",
        )
        self.assertEqual(raw.profile_requests[3][2]["page_id"], "docs")
        self.assertEqual(raw.profile_requests[5][2]["public_ref"], "https://example/web/alice/docs/")
        self.assertEqual(raw.profile_requests[8][2]["result"]["state"], "ready")
        self.assertEqual(raw.profile_requests[10][2]["project_id"], "docs")
        self.assertEqual(raw.buffers, {})

    def test_compatibility_unary_methods_use_carrier_invoke_and_projection(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = CompatibilityClient(CABICompatibilityTransport(lib, handle=7))
        base = CompatibilityCarrierBase(
            caller_ura="easynet:///r/example/agent/alice.sdk",
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="easynet:///r/example/device/dev-a",
            descriptor_version="1.0.0",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context={"form": "none"},
            auth_token="tok_example",
        )
        chat_request = {
            "model": "easynet:///r/example/ability/alice.codex.chat",
            "messages": [{"role": "user", "content": "reply with: ok"}],
        }

        models = client.list_models(CompatibilityListModelsRequest(base=base))
        chat = client.create_chat_completion(
            CompatibilityChatCompletionRequest(base=base, request=chat_request)
        )
        uploaded = client.upload_file(
            CompatibilityFileUploadRequest(
                base=base,
                purpose="batch",
                id="file-easynet-docs-1",
                file_ref="easynet:///r/example/resource/alice.files/prompt.jsonl",
                filename="prompt.jsonl",
            )
        )
        retrieved = client.retrieve_file(
            CompatibilityFileRequest(
                base=base,
                id="file-easynet-docs-1",
                file_ref="easynet:///r/example/resource/alice.files/prompt.jsonl",
                filename="prompt.jsonl",
            )
        )
        deleted = client.delete_file(
            CompatibilityFileDeleteRequest(
                base=base,
                id="file-easynet-docs-1",
                file_ref="easynet:///r/example/resource/alice.files/prompt.jsonl",
                deleted=True,
            )
        )

        self.assertEqual(
            models.data[0].id,
            "easynet:///r/example/ability/alice.codex.chat",
        )
        self.assertEqual(chat.choices[0]["message"]["content"], "ok")
        self.assertEqual(uploaded.id, "file-easynet-docs-1")
        self.assertEqual(retrieved.filename, "prompt.jsonl")
        self.assertTrue(deleted.deleted)
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            [
                "easynet_compatibility_build_list_models_invocation",
                "easynet_compatibility_project_model_page",
                "easynet_compatibility_build_chat_completion_invocation",
                "easynet_compatibility_project_chat_completion",
                "easynet_compatibility_build_file_upload_invocation",
                "easynet_compatibility_project_file_upload",
                "easynet_compatibility_build_file_retrieve_invocation",
                "easynet_compatibility_project_file",
                "easynet_compatibility_build_file_delete_invocation",
                "easynet_compatibility_project_file_delete_result",
            ],
        )
        self.assertEqual(
            [item[1]["metadata"]["system_ability"] for item in raw.runtime_requests],
            [
                "openai.list_models",
                "openai.chat_completions",
                "openai.files.upload",
                "openai.files.retrieve",
                "openai.files.delete",
            ],
        )
        self.assertEqual(raw.profile_requests[1][2]["object"], "list")
        self.assertEqual(raw.profile_requests[3][2]["id"], "chatcmpl-example")
        self.assertEqual(raw.profile_requests[5][2]["purpose"], "batch")
        self.assertEqual(raw.profile_requests[7][2]["id"], "file-easynet-docs-1")
        self.assertTrue(raw.profile_requests[9][2]["deleted"])
        self.assertEqual(raw.buffers, {})

    def test_compatibility_stream_chat_uses_runtime_stream_and_projection(self) -> None:
        raw = FakeRawCABI()
        raw.stream_events = [
            (
                b'{"sequence":1,"kind":"chunk","state":"Open","terminal":false,'
                b'"payload_json":{"id":"chatcmpl-stream-example",'
                b'"object":"chat.completion.chunk","created":1,'
                b'"model":"easynet:///r/example/ability/alice.codex.chat",'
                b'"choices":[{"index":0,"delta":{"content":"ok"},'
                b'"finish_reason":null}]}}'
            ),
            b'{"sequence":2,"kind":"terminal","state":"Completed","terminal":true}',
        ]
        lib = CLILibrary(raw)
        client = CompatibilityClient(CABICompatibilityTransport(lib, handle=7))
        base = CompatibilityCarrierBase(
            caller_ura="easynet:///r/example/agent/alice.sdk",
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="easynet:///r/example/device/dev-a",
            descriptor_version="1.0.0",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context={"form": "none"},
            auth_token="tok_example",
        )

        stream = client.stream_chat_completion(
            CompatibilityStreamChatCompletionRequest(
                base=base,
                request={
                    "model": "easynet:///r/example/ability/alice.codex.chat",
                    "messages": [{"role": "user", "content": "reply with: ok"}],
                },
            )
        )

        self.assertTrue(stream.stream)
        self.assertEqual(stream.done_sentinel, "[DONE]")
        self.assertEqual(stream.items[0].choices[0]["delta"]["content"], "ok")
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            [
                "easynet_compatibility_build_stream_chat_completion_invocation",
                "easynet_compatibility_project_chat_stream",
            ],
        )
        self.assertEqual(raw.runtime_requests[0][0], "stream_open")
        self.assertEqual(
            raw.runtime_requests[0][1]["metadata"]["system_ability"],
            "openai.chat_completions",
        )
        self.assertTrue(raw.runtime_requests[0][1]["args"]["request"]["stream"])
        self.assertEqual(
            raw.profile_requests[1][2]["chunks"][0]["id"],
            "chatcmpl-stream-example",
        )
        self.assertEqual(raw.profile_requests[1][2]["done_sentinel"], "[DONE]")
        self.assertEqual(raw.stream_closes, [404])
        self.assertEqual(raw.stream_cancels, [])
        self.assertEqual(raw.buffers, {})

    def test_compatibility_stream_chat_timeout_closes_stream(self) -> None:
        raw = FakeRawCABI()
        raw.stream_events = []
        lib = CLILibrary(raw)
        transport = CABICompatibilityTransport(lib, handle=7)
        transport.stream_recv_timeout = 0.001
        base = CompatibilityCarrierBase(
            caller_ura="easynet:///r/example/agent/alice.sdk",
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="easynet:///r/example/device/dev-a",
            descriptor_version="1.0.0",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context={"form": "none"},
            auth_token="tok_example",
        )
        request = CompatibilityStreamChatCompletionRequest(
            base=base,
            request={
                "model": "easynet:///r/example/ability/alice.codex.chat",
                "messages": [{"role": "user", "content": "reply with: ok"}],
            },
        )

        with self.assertRaises(TimeoutError):
            transport.stream_chat_completion(request.to_json_bytes())

        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            ["easynet_compatibility_build_stream_chat_completion_invocation"],
        )
        self.assertEqual(raw.runtime_requests[0][0], "stream_open")
        self.assertEqual(raw.stream_closes, [404])
        self.assertEqual(raw.stream_cancels, [])
        self.assertEqual(raw.buffers, {})

    def test_wrapper_live_helpers_use_carrier_invoke_and_projection(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = WrapperClient(CABIWrapperTransport(lib, handle=7))
        base = WrapperCarrierBase(
            caller_ura="easynet:///r/example/agent/alice.sdk",
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="easynet:///r/example/device/dev-a",
            descriptor_version="1.0.0",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context={"form": "none"},
        )

        file = client.transfer_file(
            WrapperFileTransferRequest(
                base=base,
                file=WrapperFileRecordRequest(
                    file_ref="easynet:///r/example/resource/alice.files/report.txt",
                    owner_ura="easynet:///r/example/agent/alice.sdk",
                    content_type="text/plain",
                    size_bytes=42,
                    content_hash=(
                        "sha256:"
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    ),
                ),
            )
        )
        terminal = client.start_terminal_session(
            WrapperTerminalStartRequest(
                base=base,
                session=WrapperTerminalSessionRequest(
                    session_id="term-1",
                    owner_ura="easynet:///r/example/agent/alice.sdk",
                    state="starting",
                ),
                command=("bash", "-lc"),
            )
        )
        remote = client.start_remote_desktop_session(
            WrapperRemoteDesktopStartRequest(
                base=base,
                session=WrapperRemoteDesktopSessionRequest(
                    session_id="rdp-1",
                    owner_ura="easynet:///r/example/agent/alice.sdk",
                    state="starting",
                    display_ref="display-main",
                ),
                display="main",
            )
        )
        browser = client.start_browser_session(
            WrapperBrowserStartRequest(
                base=base,
                session=WrapperBrowserSessionRequest(
                    session_id="browser-1",
                    owner_ura="easynet:///r/example/agent/alice.sdk",
                    state="starting",
                    browser_ref="browser-main",
                ),
                url="https://example.com",
            )
        )
        media = client.start_media_session(
            WrapperMediaStartRequest(
                base=base,
                session=WrapperMediaSessionRequest(
                    session_id="media-1",
                    owner_ura="easynet:///r/example/agent/alice.sdk",
                    state="starting",
                    media_kind="voice",
                    stream_ref="stream-voice-1",
                ),
                codec="opus",
            )
        )

        self.assertEqual(file.file_ref, "easynet:///r/example/resource/alice.files/report.txt")
        self.assertEqual(terminal.terminal_ref, "pty:1")
        self.assertEqual(remote.display_ref, "display:1")
        self.assertEqual(browser.browser_ref, "browser:1")
        self.assertEqual(media.stream_ref, "stream:1")
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            [
                "easynet_wrappers_build_file_transfer_invocation",
                "easynet_wrappers_project_file_record",
                "easynet_wrappers_build_terminal_session_invocation",
                "easynet_wrappers_project_terminal_session",
                "easynet_wrappers_build_remote_desktop_session_invocation",
                "easynet_wrappers_project_remote_desktop_session",
                "easynet_wrappers_build_browser_session_invocation",
                "easynet_wrappers_project_browser_session",
                "easynet_wrappers_build_media_session_invocation",
                "easynet_wrappers_project_media_session",
            ],
        )
        self.assertEqual(
            [item[1]["metadata"]["system_ability"] for item in raw.runtime_requests],
            [
                "wrapper.file.transfer",
                "wrapper.terminal.start",
                "wrapper.remote_desktop.start",
                "wrapper.browser.start",
                "wrapper.media.start",
            ],
        )
        self.assertEqual(raw.profile_requests[1][2]["content_type"], "text/plain")
        self.assertEqual(raw.profile_requests[3][2]["terminal_ref"], "pty:1")
        self.assertEqual(raw.profile_requests[5][2]["display_ref"], "display:1")
        self.assertEqual(raw.profile_requests[7][2]["browser_ref"], "browser:1")
        self.assertEqual(raw.profile_requests[9][2]["media_kind"], "voice")
        self.assertEqual(raw.buffers, {})

    def test_mission_live_methods_use_carrier_invoke_and_projection(self) -> None:
        raw = FakeRawCABI()
        lib = CLILibrary(raw)
        client = MissionClient(CABIMissionTransport(lib, handle=7))
        base = MissionCarrierBase(
            caller_ura="easynet:///r/example/agent/alice.sdk",
            callee_ura="easynet:///r/example/device/dev-a",
            subject_ura="easynet:///r/example/device/dev-a",
            descriptor_version="1.0.0",
            nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
            causal_context={"form": "none"},
        )

        run = client.run_eal(MissionRunRequest(base=base, source="mission demo"))
        run_file = client.run_file(
            "/tmp/mission.eal",
            MissionRunFileRequest(base=base, path="/tmp/mission.eal"),
        )
        tracked = client.track(MissionTrackRequest(base=base, mission_id="mission-1"))
        cancelled = client.cancel(MissionCancelRequest(base=base, mission_id="mission-1"))

        self.assertTrue(run.status.terminal)
        self.assertTrue(run_file.status.terminal)
        self.assertEqual(tracked.state, "completed")
        self.assertEqual(cancelled.state, "completed")
        self.assertEqual(
            [item[0] for item in raw.profile_requests],
            [
                "easynet_mission_build_run_eal_invocation",
                "easynet_mission_project_status",
                "easynet_mission_build_run_file_invocation",
                "easynet_mission_project_status",
                "easynet_mission_build_track_invocation",
                "easynet_mission_project_status",
                "easynet_mission_build_cancel_invocation",
                "easynet_mission_project_status",
            ],
        )
        self.assertEqual(
            [item[1]["metadata"]["system_ability"] for item in raw.runtime_requests],
            ["mission.run", "mission.run", "mission.track", "mission.cancel"],
        )
        self.assertEqual(raw.profile_requests[1][2]["run_id"], "mission-1")
        self.assertEqual(raw.profile_requests[7][2]["meta"]["status"], "cancelled")
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
