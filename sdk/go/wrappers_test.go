package easynet

import "testing"

func TestWrapperDecodesProfileRecords(t *testing.T) {
	file, err := NewWrapperFileRecordFromJSON([]byte(wrapperFileRecordJSON))
	if err != nil {
		t.Fatal(err)
	}
	if file.Kind != "file_record" || file.SizeBytes == nil || *file.SizeBytes != 42 {
		t.Fatalf("unexpected file record: %#v", file)
	}

	terminal, err := NewWrapperTerminalSessionRecordFromJSON([]byte(wrapperTerminalSessionJSON))
	if err != nil {
		t.Fatal(err)
	}
	if terminal.Kind != "terminal_session" || terminal.TerminalRef == nil {
		t.Fatalf("unexpected terminal record: %#v", terminal)
	}

	remote, err := NewWrapperRemoteDesktopSessionRecordFromJSON([]byte(wrapperRemoteDesktopSessionJSON))
	if err != nil {
		t.Fatal(err)
	}
	if remote.Kind != "remote_desktop_session" || remote.DisplayRef == nil {
		t.Fatalf("unexpected remote desktop record: %#v", remote)
	}

	browser, err := NewWrapperBrowserSessionRecordFromJSON([]byte(wrapperBrowserSessionJSON))
	if err != nil {
		t.Fatal(err)
	}
	if browser.Kind != "browser_session" || browser.State != "starting" {
		t.Fatalf("unexpected browser record: %#v", browser)
	}

	media, err := NewWrapperMediaSessionRecordFromJSON([]byte(wrapperMediaSessionJSON))
	if err != nil {
		t.Fatal(err)
	}
	if media.Kind != "media_session" || media.MediaKind != "voice" || media.StreamRef == nil {
		t.Fatalf("unexpected media record: %#v", media)
	}
}

func TestWrapperProjectsProfileRecords(t *testing.T) {
	client := NewWrapperClient()
	size := int64(42)
	file, err := client.ProjectFileRecord(WrapperFileRecordRequest{
		FileRef:     "easynet:///r/example/resource/alice.files/report.txt",
		OwnerURA:    "easynet:///r/example/agent/alice.sdk",
		ContentType: "text/plain",
		SizeBytes:   &size,
		ContentHash: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
	})
	if err != nil {
		t.Fatal(err)
	}
	if file.Metadata["source"] != "wrappers.file_record" || file.ContentHash == nil {
		t.Fatalf("unexpected projected file: %#v", file)
	}

	terminal, err := client.ProjectTerminalSession(WrapperTerminalSessionRequest{
		SessionID: "term-1",
		OwnerURA:  "easynet:///r/example/agent/alice.sdk",
		State:     "active",
	})
	if err != nil {
		t.Fatal(err)
	}
	if terminal.TerminalRef != nil || terminal.State != "active" {
		t.Fatalf("unexpected projected terminal: %#v", terminal)
	}

	remote, err := client.ProjectRemoteDesktopSession(WrapperRemoteDesktopSessionRequest{
		SessionID:  "rdp-1",
		OwnerURA:   "easynet:///r/example/agent/alice.sdk",
		State:      "active",
		DisplayRef: "display-main",
	})
	if err != nil {
		t.Fatal(err)
	}
	if remote.DisplayRef == nil || *remote.DisplayRef != "display-main" {
		t.Fatalf("unexpected projected remote desktop: %#v", remote)
	}

	browser, err := client.ProjectBrowserSession(WrapperBrowserSessionRequest{
		SessionID:  "browser-1",
		OwnerURA:   "easynet:///r/example/agent/alice.sdk",
		State:      "starting",
		BrowserRef: "browser-main",
	})
	if err != nil {
		t.Fatal(err)
	}
	if browser.BrowserRef == nil || browser.State != "starting" {
		t.Fatalf("unexpected projected browser: %#v", browser)
	}

	media, err := client.ProjectMediaSession(WrapperMediaSessionRequest{
		SessionID: "media-1",
		OwnerURA:  "easynet:///r/example/agent/alice.sdk",
		State:     "active",
		MediaKind: "voice",
		StreamRef: "stream-voice-1",
	})
	if err != nil {
		t.Fatal(err)
	}
	if media.StreamRef == nil || media.MediaKind != "voice" {
		t.Fatalf("unexpected projected media: %#v", media)
	}
}

func TestWrapperRejectsInvalidRecords(t *testing.T) {
	client := NewWrapperClient()
	if _, err := client.ProjectFileRecord(WrapperFileRecordRequest{
		FileRef:     "easynet:///r/example/resource/alice.files/report.txt",
		OwnerURA:    "not-a-ura",
		ContentType: "text/plain",
	}); err == nil {
		t.Fatal("expected invalid owner_ura rejection")
	}
	if _, err := client.ProjectTerminalSession(WrapperTerminalSessionRequest{
		SessionID: "term-1",
		OwnerURA:  "easynet:///r/example/agent/alice.sdk",
	}); err == nil {
		t.Fatal("expected missing state rejection")
	}
	if _, err := client.ProjectMediaSession(WrapperMediaSessionRequest{
		SessionID: "media-1",
		OwnerURA:  "easynet:///r/example/agent/alice.sdk",
		State:     "active",
	}); err == nil {
		t.Fatal("expected missing media_kind rejection")
	}
	if _, err := NewWrapperFileRecordFromJSON([]byte(`{"profile":"wrappers","kind":"file_record","file_ref":"x","owner_ura":"easynet:///r/example/agent/a","content_type":"text/plain","size_bytes":-1,"metadata":{}}`)); err == nil {
		t.Fatal("expected negative size rejection")
	}
}

const wrapperFileRecordJSON = `{
  "profile": "wrappers",
  "kind": "file_record",
  "file_ref": "easynet:///r/example/resource/alice.files/report.txt",
  "owner_ura": "easynet:///r/example/agent/alice.sdk",
  "content_type": "text/plain",
  "size_bytes": 42,
  "content_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "metadata": {"profile": "wrappers", "source": "wrappers.file_record"}
}`

const wrapperTerminalSessionJSON = `{
  "profile": "wrappers",
  "kind": "terminal_session",
  "session_id": "term-1",
  "owner_ura": "easynet:///r/example/agent/alice.sdk",
  "state": "active",
  "terminal_ref": "terminal-main",
  "metadata": {"profile": "wrappers", "source": "wrappers.terminal_session"}
}`

const wrapperRemoteDesktopSessionJSON = `{
  "profile": "wrappers",
  "kind": "remote_desktop_session",
  "session_id": "rdp-1",
  "owner_ura": "easynet:///r/example/agent/alice.sdk",
  "state": "active",
  "display_ref": "display-main",
  "metadata": {"profile": "wrappers", "source": "wrappers.remote_desktop_session"}
}`

const wrapperBrowserSessionJSON = `{
  "profile": "wrappers",
  "kind": "browser_session",
  "session_id": "browser-1",
  "owner_ura": "easynet:///r/example/agent/alice.sdk",
  "state": "starting",
  "browser_ref": "browser-main",
  "metadata": {"profile": "wrappers", "source": "wrappers.browser_session"}
}`

const wrapperMediaSessionJSON = `{
  "profile": "wrappers",
  "kind": "media_session",
  "session_id": "media-1",
  "owner_ura": "easynet:///r/example/agent/alice.sdk",
  "state": "active",
  "media_kind": "voice",
  "stream_ref": "stream-voice-1",
  "metadata": {"profile": "wrappers", "source": "wrappers.media_session"}
}`
