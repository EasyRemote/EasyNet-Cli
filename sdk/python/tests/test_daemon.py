import json
import unittest

from easynet_sdk import (
    AttachOptions,
    AbilityInvocationClient,
    AdminClient,
    CompatibilityClient,
    ConnectOptions,
    DaemonControl,
    DaemonLifecycleFacade,
    DaemonLifecycleState,
    DaemonMode,
    DaemonStartProjection,
    DiscoverOptions,
    DirectoryClient,
    ErrorCode,
    EventClient,
    HealthClient,
    HostBindingClient,
    IdentityClient,
    MissionClient,
    PublicationClient,
    ReceiptClient,
    SDKError,
    StartConfig,
    StopOptions,
    SurfaceClient,
    WrapperClient,
    attach_daemon,
    connect_local,
    discover_daemon,
    is_code,
    start_daemon,
)


class MemoryRuntimeTransport:
    def close(self) -> None:
        pass

    def invoke(self, draft_json: bytes) -> bytes:
        raise RuntimeError("not used")


class MemoryProfileTransport:
    def __init__(self, profile: str) -> None:
        self.profile = profile
        self.closed = False

    def close(self) -> None:
        self.closed = True


class MemoryDaemonTransport:
    def __init__(self) -> None:
        self.discover_json = (
            b'{"control_endpoint":"unix:///tmp/control.sock",'
            b'"invocation_endpoint":"unix:///tmp/daemon.sock"}'
        )
        self.start_json = ready_status()
        self.attach_json = ready_status()
        self.status_json = ready_status()
        self.stop_json = b'{"handle_id":"daemon-1","state":"Stopped","mode":"hub"}'
        self.start_calls = 0
        self.stop_calls = 0
        self.detach_calls = 0
        self.open_calls = 0
        self.invocation_endpoint_calls = 0
        self.profile_opens: list[tuple[str, dict[str, object]]] = []
        self.open_error: Exception | None = None
        self.seen_start: dict[str, object] | None = None
        self.seen_options: dict[str, object] | None = None

    def discover(self, options_json: bytes) -> bytes:
        self.seen_options = json.loads(options_json.decode("utf-8"))
        return self.discover_json

    def start(self, config_json: bytes) -> bytes:
        self.start_calls += 1
        self.seen_start = json.loads(config_json.decode("utf-8"))
        return self.start_json

    def attach(self, options_json: bytes) -> bytes:
        self.seen_options = json.loads(options_json.decode("utf-8"))
        return self.attach_json

    def status(self, handle_id: str) -> bytes:
        return self.status_json

    def invocation_endpoint(self, handle_id: str) -> str:
        self.invocation_endpoint_calls += 1
        return "unix:///tmp/live-daemon.sock"

    def open_runtime(self, handle_id: str, options_json: bytes):
        self.open_calls += 1
        self.seen_options = json.loads(options_json.decode("utf-8"))
        if self.open_error is not None:
            raise self.open_error
        return MemoryRuntimeTransport(), b'{"ready":true}'

    def open_runtime_transport(self, handle_id: str, options_json: bytes):
        return self._open_profile_transport("runtime", options_json)

    def open_profile(self, handle_id: str, profile: str, options_json: bytes):
        return self._open_profile_transport(profile, options_json)

    def open_directory_transport(self, handle_id: str, options_json: bytes):
        return self._open_profile_transport("directory", options_json)

    def open_identity_transport(self, handle_id: str, options_json: bytes):
        return self._open_profile_transport("identity", options_json)

    def open_receipt_transport(self, handle_id: str, options_json: bytes):
        return self._open_profile_transport("receipt", options_json)

    def open_publication_transport(self, handle_id: str, options_json: bytes):
        return self._open_profile_transport("publication", options_json)

    def open_host_binding_transport(self, handle_id: str, options_json: bytes):
        return self._open_profile_transport("host_binding", options_json)

    def open_mission_transport(self, handle_id: str, options_json: bytes):
        return self._open_profile_transport("mission", options_json)

    def open_admin_transport(self, handle_id: str, options_json: bytes):
        return self._open_profile_transport("admin", options_json)

    def open_events_transport(self, handle_id: str, options_json: bytes):
        return self._open_profile_transport("events", options_json)

    def open_surface_transport(self, handle_id: str, options_json: bytes):
        return self._open_profile_transport("surface", options_json)

    def open_compatibility_transport(self, handle_id: str, options_json: bytes):
        return self._open_profile_transport("compatibility", options_json)

    def open_wrapper_transport(self, handle_id: str, options_json: bytes):
        return self._open_profile_transport("wrapper", options_json)

    def _open_profile_transport(self, profile: str, options_json: bytes):
        self.profile_opens.append((profile, json.loads(options_json.decode("utf-8"))))
        if profile == "runtime":
            return MemoryRuntimeTransport()
        return MemoryProfileTransport(profile)

    def stop(self, handle_id: str, options_json: bytes) -> bytes:
        self.stop_calls += 1
        return self.stop_json

    def detach(self, handle_id: str) -> None:
        self.detach_calls += 1


class LifecycleOnlyDaemonTransport:
    def __init__(self) -> None:
        self.inner = MemoryDaemonTransport()

    def discover(self, options_json: bytes) -> bytes:
        return self.inner.discover(options_json)

    def start(self, config_json: bytes) -> bytes:
        return self.inner.start(config_json)

    def attach(self, options_json: bytes) -> bytes:
        return self.inner.attach(options_json)

    def status(self, handle_id: str) -> bytes:
        return self.inner.status(handle_id)

    def invocation_endpoint(self, handle_id: str) -> str:
        return self.inner.invocation_endpoint(handle_id)

    def open_runtime(self, handle_id: str, options_json: bytes):
        return self.inner.open_runtime(handle_id, options_json)

    def stop(self, handle_id: str, options_json: bytes) -> bytes:
        return self.inner.stop(handle_id, options_json)

    def detach(self, handle_id: str) -> None:
        self.inner.detach(handle_id)


def ready_status() -> bytes:
    return (
        b'{"handle_id":"daemon-1","state":"Running","mode":"hub","pid":42,'
        b'"endpoints":{"control_endpoint":"unix:///tmp/control.sock",'
        b'"invocation_endpoint":"unix:///tmp/daemon.sock",'
        b'"public_endpoint":"https://hub.example"}}'
    )


class DaemonTests(unittest.TestCase):
    def test_daemon_start_projection_preserves_wire_shape(self) -> None:
        config = DaemonStartProjection.hub(
            " acme ",
            env={"RUST_LOG": "info"},
            log_path="/tmp/daemon.log",
            detached=True,
        )
        foreground = DaemonStartProjection.device(
            "dev-a",
            detached=False,
        )

        self.assertEqual(
            config.to_wire_dict(),
            {
                "mode": "hub",
                "realm": "acme",
                "env": {"RUST_LOG": "info"},
                "log_path": "/tmp/daemon.log",
                "detached": True,
            },
        )
        self.assertEqual(
            foreground.to_wire_dict(),
            {"mode": "device", "device_id": "dev-a", "detached": False},
        )
        self.assertNotIn(
            "detached",
            DaemonStartProjection.device("dev-a").to_wire_dict(),
        )

    def test_daemon_start_projection_projects_to_runtime_core_start_config(self) -> None:
        config = DaemonStartProjection.device(
            " dev-a ",
            env={"RUST_LOG": "debug"},
            log_path="/tmp/daemon.log",
            detached=True,
        ).to_start_config()

        self.assertEqual(config.mode, DaemonMode.DEVICE)
        self.assertEqual(config.device_id, "dev-a")
        self.assertEqual(config.realm, "")
        self.assertEqual(config.log_path, "/tmp/daemon.log")
        self.assertTrue(config.detached)
        self.assertEqual(config.env, {"RUST_LOG": "debug"})

    def test_daemon_start_projection_rejects_invalid_inputs(self) -> None:
        with self.assertRaises(SDKError) as missing_realm:
            DaemonStartProjection.hub(" ")
        self.assertEqual(missing_realm.exception.details["reason"], "empty_realm")

        with self.assertRaises(SDKError) as missing_node:
            DaemonStartProjection.device()
        self.assertEqual(missing_node.exception.details["reason"], "missing_device_id")

        with self.assertRaises(SDKError) as invalid_mode:
            DaemonStartProjection.from_profile(mode="both", realm="acme")
        self.assertEqual(invalid_mode.exception.details["reason"], "invalid_daemon_mode")

    def test_daemon_lifecycle_facade_starts_and_projects_status(self) -> None:
        transport = MemoryDaemonTransport()
        facade = DaemonLifecycleFacade(DaemonControl(transport))

        handle = facade.start(DaemonStartProjection.hub("acme"))
        status = handle.status_dict()

        assert transport.seen_start is not None
        self.assertEqual(transport.seen_start["mode"], "hub")
        self.assertEqual(transport.seen_start["realm"], "acme")
        self.assertEqual(status["state"], "Running")
        self.assertEqual(status["handle_id"], "daemon-1")
        self.assertEqual(status["mode"], "hub")
        self.assertEqual(status["pid"], 42)
        endpoints = status["endpoints"]
        self.assertIsInstance(endpoints, dict)
        self.assertEqual(endpoints["invocation_endpoint"], "unix:///tmp/daemon.sock")

    def test_daemon_handle_facade_opens_transport_adapter(self) -> None:
        transport = MemoryDaemonTransport()
        facade = DaemonLifecycleFacade(DaemonControl(transport))
        handle = facade.start(DaemonStartProjection.hub("acme"))

        adapter = handle.open_transport_adapter()

        self.assertEqual(transport.open_calls, 1)
        self.assertTrue(hasattr(adapter, "invoke"))

    def test_start_returns_runtime_ready_handle(self) -> None:
        transport = MemoryDaemonTransport()

        handle = start_daemon(
            transport,
            StartConfig(
                mode=DaemonMode.HUB,
                listen_tcp="127.0.0.1:9443",
                tls_cert_path="/tmp/cert.pem",
                tls_key_path="/tmp/key.pem",
            ),
        )

        self.assertEqual(handle.handle_id, "daemon-1")
        self.assertEqual(handle.state, DaemonLifecycleState.RUNNING)
        assert transport.seen_start is not None
        self.assertEqual(transport.seen_start["listen_tcp"], "127.0.0.1:9443")
        self.assertEqual(handle.endpoints.invocation_endpoint, "unix:///tmp/daemon.sock")

    def test_handle_invocation_endpoint_uses_lifecycle_transport(self) -> None:
        transport = MemoryDaemonTransport()
        handle = start_daemon(transport, StartConfig(mode=DaemonMode.HUB))

        endpoint = handle.invocation_endpoint()

        self.assertEqual(endpoint, "unix:///tmp/live-daemon.sock")
        self.assertEqual(handle.endpoints.invocation_endpoint, endpoint)
        self.assertEqual(transport.invocation_endpoint_calls, 1)

        handle.detach()
        with self.assertRaises(SDKError) as caught:
            handle.invocation_endpoint()
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_HANDLE))

    def test_start_rejects_unsafe_mode_policy_before_transport(self) -> None:
        transport = MemoryDaemonTransport()

        with self.assertRaises(SDKError):
            start_daemon(
                transport,
                StartConfig(mode=DaemonMode.DEVICE, listen_tcp="0.0.0.0:9443"),
            )
        self.assertEqual(transport.start_calls, 0)

        with self.assertRaises(SDKError):
            start_daemon(
                transport,
                StartConfig(mode=DaemonMode.HUB, listen_tcp="0.0.0.0:9443"),
            )
        self.assertEqual(transport.start_calls, 0)

    def test_attach_rejects_control_only_readiness(self) -> None:
        transport = MemoryDaemonTransport()
        transport.attach_json = (
            b'{"handle_id":"daemon-1","state":"ControlOnly",'
            b'"endpoints":{"control_endpoint":"unix:///tmp/control.sock"}}'
        )

        with self.assertRaises(SDKError) as caught:
            attach_daemon(
                transport,
                AttachOptions(control_endpoint="unix:///tmp/control.sock"),
            )

        self.assertTrue(is_code(caught.exception, ErrorCode.CONTROL_ONLY))

    def test_discover_preserves_advertised_invocation_endpoint(self) -> None:
        transport = MemoryDaemonTransport()
        transport.discover_json = (
            b'{"control_endpoint":"unix:///tmp/control.sock",'
            b'"invocation_endpoint":"unix:///tmp/custom-daemon.sock"}'
        )

        endpoints = discover_daemon(transport, DiscoverOptions(home_dir="/tmp/home"))

        self.assertEqual(
            endpoints.invocation_endpoint,
            "unix:///tmp/custom-daemon.sock",
        )

    def test_open_runtime_requires_ready_state(self) -> None:
        transport = MemoryDaemonTransport()
        handle = start_daemon(transport, StartConfig(mode=DaemonMode.HUB))

        client = handle.open_runtime(ConnectOptions(max_message_bytes=4096))

        self.assertIsNotNone(client)
        self.assertEqual(transport.open_calls, 1)
        assert transport.seen_options is not None
        self.assertEqual(transport.seen_options["max_message_bytes"], 4096)

        handle._status = handle._status.__class__(
            state=DaemonLifecycleState.CONTROL_READY,
            handle_id=handle.handle_id,
        )
        with self.assertRaises(SDKError):
            handle.open_runtime()

    def test_profile_factories_return_public_clients_from_daemon_handle(self) -> None:
        transport = MemoryDaemonTransport()
        handle = start_daemon(transport, StartConfig(mode=DaemonMode.HUB))
        options = ConnectOptions(max_message_bytes=4096)

        clients = (
            handle.directory(options),
            handle.identity(options),
            handle.receipts(options),
            handle.publication(options),
            handle.host_binding(options),
            handle.missions(options),
            handle.admin(options),
            handle.events(options),
            handle.surfaces(options),
            handle.compatibility(options),
            handle.wrappers(options),
            handle.health(options),
            handle.ability_invocation(options),
        )

        self.assertIsInstance(clients[0], DirectoryClient)
        self.assertIsInstance(clients[1], IdentityClient)
        self.assertIsInstance(clients[2], ReceiptClient)
        self.assertIsInstance(clients[3], PublicationClient)
        self.assertIsInstance(clients[4], HostBindingClient)
        self.assertIsInstance(clients[5], MissionClient)
        self.assertIsInstance(clients[6], AdminClient)
        self.assertIsInstance(clients[7], EventClient)
        self.assertIsInstance(clients[8], SurfaceClient)
        self.assertIsInstance(clients[9], CompatibilityClient)
        self.assertIsInstance(clients[10], WrapperClient)
        self.assertIsInstance(clients[11], HealthClient)
        self.assertIsInstance(clients[12], AbilityInvocationClient)
        self.assertEqual(
            [profile for profile, _ in transport.profile_opens],
            [
                "directory",
                "identity",
                "receipt",
                "publication",
                "runtime",
                "host_binding",
                "mission",
                "admin",
                "events",
                "surface",
                "compatibility",
                "wrapper",
                "runtime",
                "runtime",
                "identity",
            ],
        )
        self.assertTrue(
            all(options["max_message_bytes"] == 4096 for _, options in transport.profile_opens)
        )

        handle.detach()
        with self.assertRaises(SDKError) as caught:
            handle.directory()
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_HANDLE))

    def test_profile_factories_require_typed_transport_capability(self) -> None:
        transport = LifecycleOnlyDaemonTransport()
        handle = start_daemon(transport, StartConfig(mode=DaemonMode.HUB))

        with self.assertRaises(SDKError) as caught:
            handle.directory()

        self.assertTrue(is_code(caught.exception, ErrorCode.NOT_IMPLEMENTED))

    def test_connect_local_discovers_attaches_opens_and_detaches(self) -> None:
        transport = MemoryDaemonTransport()
        transport.discover_json = (
            b'{"control_endpoint":"unix:///tmp/control.sock",'
            b'"invocation_endpoint":"unix:///tmp/discovered-daemon.sock"}'
        )

        client = connect_local(
            transport,
            ConnectOptions(control_path="/tmp/control.sock", max_message_bytes=4096),
        )

        self.assertIsNotNone(client)
        self.assertEqual(transport.open_calls, 1)
        self.assertEqual(transport.detach_calls, 1)
        assert transport.seen_options is not None
        self.assertEqual(
            transport.seen_options["endpoint"], "unix:///tmp/discovered-daemon.sock"
        )
        self.assertEqual(transport.seen_options["max_message_bytes"], 4096)

    def test_connect_local_rejects_control_only_attach(self) -> None:
        transport = MemoryDaemonTransport()
        transport.attach_json = (
            b'{"handle_id":"daemon-1","state":"ControlOnly",'
            b'"endpoints":{"control_endpoint":"unix:///tmp/control.sock"}}'
        )

        with self.assertRaises(SDKError) as caught:
            connect_local(transport, ConnectOptions())

        self.assertTrue(is_code(caught.exception, ErrorCode.CONTROL_ONLY))
        self.assertEqual(transport.open_calls, 0)
        self.assertEqual(transport.detach_calls, 0)

    def test_connect_local_detaches_after_open_runtime_failure(self) -> None:
        transport = MemoryDaemonTransport()
        transport.open_error = RuntimeError("open failed")

        with self.assertRaises(SDKError) as caught:
            connect_local(transport, ConnectOptions())

        self.assertTrue(is_code(caught.exception, ErrorCode.TRANSPORT))
        self.assertEqual(transport.open_calls, 1)
        self.assertEqual(transport.detach_calls, 1)

    def test_connect_local_allows_explicit_endpoint_override(self) -> None:
        transport = MemoryDaemonTransport()
        transport.discover_json = (
            b'{"control_endpoint":"unix:///tmp/control.sock",'
            b'"invocation_endpoint":"unix:///tmp/discovered-daemon.sock"}'
        )

        connect_local(
            transport,
            ConnectOptions(endpoint="unix:///tmp/explicit-daemon.sock"),
        )

        assert transport.seen_options is not None
        self.assertEqual(
            transport.seen_options["endpoint"], "unix:///tmp/explicit-daemon.sock"
        )

    def test_stop_is_idempotent_and_detach_does_not_stop(self) -> None:
        transport = MemoryDaemonTransport()
        handle = start_daemon(transport, StartConfig(mode=DaemonMode.HUB))

        handle.stop(StopOptions())
        handle.stop(StopOptions())
        self.assertEqual(transport.stop_calls, 1)

        handle.detach()
        self.assertEqual(transport.detach_calls, 1)
        self.assertEqual(transport.stop_calls, 1)
        with self.assertRaises(SDKError) as caught:
            handle.status()
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_HANDLE))


if __name__ == "__main__":
    unittest.main()
