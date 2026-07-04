import json
import unittest

from easynet_sdk import (
    ErrorCode,
    SDKError,
    WrapperBrowserSessionRecord,
    WrapperBrowserSessionRequest,
    WrapperBrowserStartRequest,
    WrapperCarrierBase,
    WrapperClient,
    WrapperFileRecord,
    WrapperFileRecordRequest,
    WrapperFileTransferRequest,
    WrapperMediaSessionRecord,
    WrapperMediaSessionRequest,
    WrapperMediaStartRequest,
    WrapperRemoteDesktopSessionRecord,
    WrapperRemoteDesktopSessionRequest,
    WrapperRemoteDesktopStartRequest,
    WrapperTerminalSessionRecord,
    WrapperTerminalSessionRequest,
    WrapperTerminalStartRequest,
    is_code,
)


FILE_RECORD_JSON = b"""
{
  "profile": "wrappers",
  "kind": "file_record",
  "file_ref": "easynet:///r/example/resource/alice.files/report.txt",
  "owner_ura": "easynet:///r/example/agent/alice.sdk",
  "content_type": "text/plain",
  "size_bytes": 42,
  "content_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "metadata": {"profile": "wrappers", "source": "wrappers.file_record"}
}
"""

TERMINAL_SESSION_JSON = b"""
{
  "profile": "wrappers",
  "kind": "terminal_session",
  "session_id": "term-1",
  "owner_ura": "easynet:///r/example/agent/alice.sdk",
  "state": "active",
  "terminal_ref": "terminal-main",
  "metadata": {"profile": "wrappers", "source": "wrappers.terminal_session"}
}
"""

REMOTE_DESKTOP_SESSION_JSON = b"""
{
  "profile": "wrappers",
  "kind": "remote_desktop_session",
  "session_id": "rdp-1",
  "owner_ura": "easynet:///r/example/agent/alice.sdk",
  "state": "active",
  "display_ref": "display-main",
  "metadata": {"profile": "wrappers", "source": "wrappers.remote_desktop_session"}
}
"""

BROWSER_SESSION_JSON = b"""
{
  "profile": "wrappers",
  "kind": "browser_session",
  "session_id": "browser-1",
  "owner_ura": "easynet:///r/example/agent/alice.sdk",
  "state": "starting",
  "browser_ref": "browser-main",
  "metadata": {"profile": "wrappers", "source": "wrappers.browser_session"}
}
"""

MEDIA_SESSION_JSON = b"""
{
  "profile": "wrappers",
  "kind": "media_session",
  "session_id": "media-1",
  "owner_ura": "easynet:///r/example/agent/alice.sdk",
  "state": "active",
  "media_kind": "voice",
  "stream_ref": "stream-voice-1",
  "metadata": {"profile": "wrappers", "source": "wrappers.media_session"}
}
"""


FILE_INVOCATION_JSON = b"""
{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.wrapper.file.transfer@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"wrapper_kind": "file", "operation": "transfer"},
  "content_type": "application/json",
  "metadata": {"request_id": "wrapper-test", "profile": "wrappers", "system_ability": "wrapper.file.transfer", "carrier_owner": "daemon_sdk"}
}
"""

TERMINAL_INVOCATION_JSON = b"""
{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.wrapper.terminal.start@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"wrapper_kind": "terminal", "session_id": "term-1"},
  "content_type": "application/json",
  "metadata": {"request_id": "wrapper-test", "profile": "wrappers", "system_ability": "wrapper.terminal.start", "carrier_owner": "daemon_sdk"}
}
"""

REMOTE_DESKTOP_INVOCATION_JSON = b"""
{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.wrapper.remote_desktop.start@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"wrapper_kind": "remote_desktop", "session_id": "rdp-1"},
  "content_type": "application/json",
  "metadata": {"request_id": "wrapper-test", "profile": "wrappers", "system_ability": "wrapper.remote_desktop.start", "carrier_owner": "daemon_sdk"}
}
"""

BROWSER_INVOCATION_JSON = b"""
{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.wrapper.browser.start@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"wrapper_kind": "browser", "session_id": "browser-1"},
  "content_type": "application/json",
  "metadata": {"request_id": "wrapper-test", "profile": "wrappers", "system_ability": "wrapper.browser.start", "carrier_owner": "daemon_sdk"}
}
"""

MEDIA_INVOCATION_JSON = b"""
{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.wrapper.media.start@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"wrapper_kind": "media", "session_id": "media-1"},
  "content_type": "application/json",
  "metadata": {"request_id": "wrapper-test", "profile": "wrappers", "system_ability": "wrapper.media.start", "carrier_owner": "daemon_sdk"}
}
"""


class MemoryWrapperTransport:
    def __init__(self) -> None:
        self.seen: dict[str, dict[str, object]] = {}
        self.close_calls = 0

    def _remember(self, name: str, request_json: bytes) -> None:
        self.seen[name] = json.loads(request_json.decode("utf-8"))

    def build_file_transfer_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_file", request_json)
        return FILE_INVOCATION_JSON

    def build_terminal_session_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_terminal", request_json)
        return TERMINAL_INVOCATION_JSON

    def build_remote_desktop_session_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_remote_desktop", request_json)
        return REMOTE_DESKTOP_INVOCATION_JSON

    def build_browser_session_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_browser", request_json)
        return BROWSER_INVOCATION_JSON

    def build_media_session_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_media", request_json)
        return MEDIA_INVOCATION_JSON

    def transfer_file(self, request_json: bytes) -> bytes:
        self._remember("transfer_file", request_json)
        return FILE_RECORD_JSON

    def start_terminal_session(self, request_json: bytes) -> bytes:
        self._remember("start_terminal", request_json)
        return TERMINAL_SESSION_JSON

    def start_remote_desktop_session(self, request_json: bytes) -> bytes:
        self._remember("start_remote_desktop", request_json)
        return REMOTE_DESKTOP_SESSION_JSON

    def start_browser_session(self, request_json: bytes) -> bytes:
        self._remember("start_browser", request_json)
        return BROWSER_SESSION_JSON

    def start_media_session(self, request_json: bytes) -> bytes:
        self._remember("start_media", request_json)
        return MEDIA_SESSION_JSON

    def close(self) -> None:
        self.close_calls += 1


def wrapper_base() -> WrapperCarrierBase:
    return WrapperCarrierBase(
        caller_ura="easynet:///r/example/agent/alice.sdk",
        callee_ura="easynet:///r/example/device/dev-a",
        subject_ura="easynet:///r/example/device/dev-a",
        descriptor_version="1.0.0",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        metadata={"request_id": "wrapper-test"},
    )


def file_transfer_request() -> WrapperFileTransferRequest:
    return WrapperFileTransferRequest(
        wrapper_base(),
        WrapperFileRecordRequest(
            file_ref="easynet:///r/example/resource/alice.files/report.txt",
            owner_ura="easynet:///r/example/agent/alice.sdk",
            content_type="text/plain",
            size_bytes=42,
            content_hash="sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            metadata={"route": "upload"},
        ),
    )


def terminal_start_request() -> WrapperTerminalStartRequest:
    return WrapperTerminalStartRequest(
        wrapper_base(),
        WrapperTerminalSessionRequest(
            session_id="term-1",
            owner_ura="easynet:///r/example/agent/alice.sdk",
            state="starting",
        ),
        command=("bash", "-lc"),
    )


def remote_desktop_start_request() -> WrapperRemoteDesktopStartRequest:
    return WrapperRemoteDesktopStartRequest(
        wrapper_base(),
        WrapperRemoteDesktopSessionRequest(
            session_id="rdp-1",
            owner_ura="easynet:///r/example/agent/alice.sdk",
            state="starting",
            display_ref="display-main",
        ),
        display="main",
    )


def browser_start_request() -> WrapperBrowserStartRequest:
    return WrapperBrowserStartRequest(
        wrapper_base(),
        WrapperBrowserSessionRequest(
            session_id="browser-1",
            owner_ura="easynet:///r/example/agent/alice.sdk",
            state="starting",
            browser_ref="browser-main",
        ),
        url="https://example.com",
    )


def media_start_request() -> WrapperMediaStartRequest:
    return WrapperMediaStartRequest(
        wrapper_base(),
        WrapperMediaSessionRequest(
            session_id="media-1",
            owner_ura="easynet:///r/example/agent/alice.sdk",
            state="starting",
            media_kind="voice",
            stream_ref="stream-voice-1",
        ),
        codec="opus",
    )


class WrapperClientTests(unittest.TestCase):
    def test_builds_execution_invocations(self) -> None:
        transport = MemoryWrapperTransport()
        client = WrapperClient(transport)

        file_draft = client.build_file_transfer_invocation(file_transfer_request())
        self.assertEqual(
            file_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.wrapper.file.transfer@1.0.0",
        )
        self.assertEqual(transport.seen["build_file"]["wrapper_kind"], "file")
        self.assertEqual(transport.seen["build_file"]["operation"], "transfer")
        self.assertEqual(transport.seen["build_file"]["metadata"]["request_id"], "wrapper-test")
        self.assertEqual(transport.seen["build_file"]["metadata"]["route"], "upload")

        terminal_draft = client.build_terminal_session_invocation(terminal_start_request())
        self.assertEqual(
            terminal_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.wrapper.terminal.start@1.0.0",
        )
        remote_draft = client.build_remote_desktop_session_invocation(remote_desktop_start_request())
        self.assertEqual(
            remote_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.wrapper.remote_desktop.start@1.0.0",
        )
        browser_draft = client.build_browser_session_invocation(browser_start_request())
        self.assertEqual(
            browser_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.wrapper.browser.start@1.0.0",
        )
        media_draft = client.build_media_session_invocation(media_start_request())
        self.assertEqual(
            media_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.wrapper.media.start@1.0.0",
        )

    def test_executes_transport_backed_helpers(self) -> None:
        client = WrapperClient(MemoryWrapperTransport())

        file = client.transfer_file(file_transfer_request())
        self.assertEqual(file.kind, "file_record")
        terminal = client.start_terminal_session(terminal_start_request())
        self.assertEqual(terminal.kind, "terminal_session")
        remote = client.start_remote_desktop_session(remote_desktop_start_request())
        self.assertEqual(remote.kind, "remote_desktop_session")
        browser = client.start_browser_session(browser_start_request())
        self.assertEqual(browser.kind, "browser_session")
        media = client.start_media_session(media_start_request())
        self.assertEqual(media.kind, "media_session")

    def test_decodes_profile_records(self) -> None:
        file = WrapperFileRecord.from_json(FILE_RECORD_JSON)
        self.assertEqual(file.kind, "file_record")
        self.assertEqual(file.size_bytes, 42)

        terminal = WrapperTerminalSessionRecord.from_json(TERMINAL_SESSION_JSON)
        self.assertEqual(terminal.terminal_ref, "terminal-main")

        remote = WrapperRemoteDesktopSessionRecord.from_json(REMOTE_DESKTOP_SESSION_JSON)
        self.assertEqual(remote.display_ref, "display-main")

        browser = WrapperBrowserSessionRecord.from_json(BROWSER_SESSION_JSON)
        self.assertEqual(browser.state, "starting")

        media = WrapperMediaSessionRecord.from_json(MEDIA_SESSION_JSON)
        self.assertEqual(media.media_kind, "voice")
        self.assertEqual(media.stream_ref, "stream-voice-1")

    def test_projects_profile_records(self) -> None:
        client = WrapperClient()
        file = client.project_file_record(
            WrapperFileRecordRequest(
                file_ref="easynet:///r/example/resource/alice.files/report.txt",
                owner_ura="easynet:///r/example/agent/alice.sdk",
                content_type="text/plain",
                size_bytes=42,
                content_hash="sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
        )
        self.assertEqual(file.metadata["source"], "wrappers.file_record")
        self.assertEqual(file.content_hash, "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")

        terminal = client.project_terminal_session(
            WrapperTerminalSessionRequest(
                session_id="term-1",
                owner_ura="easynet:///r/example/agent/alice.sdk",
                state="active",
            )
        )
        self.assertIsNone(terminal.terminal_ref)

        remote = client.project_remote_desktop_session(
            WrapperRemoteDesktopSessionRequest(
                session_id="rdp-1",
                owner_ura="easynet:///r/example/agent/alice.sdk",
                state="active",
                display_ref="display-main",
            )
        )
        self.assertEqual(remote.display_ref, "display-main")

        browser = client.project_browser_session(
            WrapperBrowserSessionRequest(
                session_id="browser-1",
                owner_ura="easynet:///r/example/agent/alice.sdk",
                state="starting",
                browser_ref="browser-main",
            )
        )
        self.assertEqual(browser.browser_ref, "browser-main")

        media = client.project_media_session(
            WrapperMediaSessionRequest(
                session_id="media-1",
                owner_ura="easynet:///r/example/agent/alice.sdk",
                state="active",
                media_kind="voice",
                stream_ref="stream-voice-1",
            )
        )
        self.assertEqual(media.stream_ref, "stream-voice-1")

    def test_rejects_invalid_records(self) -> None:
        client = WrapperClient()
        with self.assertRaises(SDKError):
            client.transfer_file(file_transfer_request())
        with self.assertRaises(SDKError):
            client.project_file_record(
                WrapperFileRecordRequest(
                    file_ref="easynet:///r/example/resource/alice.files/report.txt",
                    owner_ura="not-a-ura",
                    content_type="text/plain",
                )
            )
        with self.assertRaises(SDKError):
            client.project_terminal_session(
                WrapperTerminalSessionRequest(
                    session_id="term-1",
                    owner_ura="easynet:///r/example/agent/alice.sdk",
                    state="",
                )
            )
        with self.assertRaises(SDKError):
            client.project_media_session(
                WrapperMediaSessionRequest(
                    session_id="media-1",
                    owner_ura="easynet:///r/example/agent/alice.sdk",
                    state="active",
                    media_kind="",
                )
            )
        with self.assertRaises(SDKError):
            WrapperFileRecord.from_json(
                b'{"profile":"wrappers","kind":"file_record","file_ref":"x","owner_ura":"easynet:///r/example/agent/a","content_type":"text/plain","size_bytes":-1,"metadata":{}}'
            )

    def test_close_delegates_once_and_fails_closed(self) -> None:
        transport = MemoryWrapperTransport()
        client = WrapperClient(transport)

        client.close()
        client.close()

        self.assertEqual(transport.close_calls, 1)
        with self.assertRaises(SDKError) as caught:
            client.build_file_transfer_invocation(file_transfer_request())
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(transport.seen, {})

        projected = client.project_file_record(
            WrapperFileRecordRequest(
                file_ref="easynet:///r/example/resource/alice.files/report.txt",
                owner_ura="easynet:///r/example/agent/alice.sdk",
                content_type="text/plain",
                size_bytes=42,
            )
        )
        self.assertEqual(projected.kind, "file_record")


if __name__ == "__main__":
    unittest.main()
