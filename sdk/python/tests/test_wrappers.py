import unittest

from easynet_sdk import (
    SDKError,
    WrapperBrowserSessionRecord,
    WrapperBrowserSessionRequest,
    WrapperClient,
    WrapperFileRecord,
    WrapperFileRecordRequest,
    WrapperMediaSessionRecord,
    WrapperMediaSessionRequest,
    WrapperRemoteDesktopSessionRecord,
    WrapperRemoteDesktopSessionRequest,
    WrapperTerminalSessionRecord,
    WrapperTerminalSessionRequest,
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


class WrapperClientTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
