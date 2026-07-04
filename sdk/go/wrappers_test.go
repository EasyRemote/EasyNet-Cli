package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

type memoryWrapperTransport struct {
	seen              map[string]map[string]any
	fileInvocation    string
	termInvocation    string
	rdpInvocation     string
	browserInvocation string
	mediaInvocation   string
}

func (m *memoryWrapperTransport) remember(name string, requestJSON []byte) {
	if m.seen == nil {
		m.seen = map[string]map[string]any{}
	}
	var decoded map[string]any
	_ = json.Unmarshal(requestJSON, &decoded)
	m.seen[name] = decoded
}

func (m *memoryWrapperTransport) BuildFileTransferInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_file", requestJSON)
	return []byte(m.fileInvocation), nil
}

func (m *memoryWrapperTransport) BuildTerminalSessionInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_terminal", requestJSON)
	return []byte(m.termInvocation), nil
}

func (m *memoryWrapperTransport) BuildRemoteDesktopSessionInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_remote_desktop", requestJSON)
	return []byte(m.rdpInvocation), nil
}

func (m *memoryWrapperTransport) BuildBrowserSessionInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_browser", requestJSON)
	return []byte(m.browserInvocation), nil
}

func (m *memoryWrapperTransport) BuildMediaSessionInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_media", requestJSON)
	return []byte(m.mediaInvocation), nil
}

func (m *memoryWrapperTransport) TransferFile(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("transfer_file", requestJSON)
	return []byte(wrapperFileRecordJSON), nil
}

func (m *memoryWrapperTransport) StartTerminalSession(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("start_terminal", requestJSON)
	return []byte(wrapperTerminalSessionJSON), nil
}

func (m *memoryWrapperTransport) StartRemoteDesktopSession(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("start_remote_desktop", requestJSON)
	return []byte(wrapperRemoteDesktopSessionJSON), nil
}

func (m *memoryWrapperTransport) StartBrowserSession(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("start_browser", requestJSON)
	return []byte(wrapperBrowserSessionJSON), nil
}

func (m *memoryWrapperTransport) StartMediaSession(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("start_media", requestJSON)
	return []byte(wrapperMediaSessionJSON), nil
}

func wrapperBaseForTest() WrapperCarrierBase {
	return WrapperCarrierBase{
		CallerURA:         "easynet:///r/example/agent/alice.sdk",
		CalleeURA:         "easynet:///r/example/device/dev-a",
		SubjectURA:        "easynet:///r/example/device/dev-a",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		Metadata:          map[string]any{"request_id": "wrapper-test"},
	}
}

func wrapperFileTransferRequest() WrapperFileTransferRequest {
	size := int64(42)
	return WrapperFileTransferRequest{
		WrapperCarrierBase: wrapperBaseForTest(),
		WrapperFileRecordRequest: WrapperFileRecordRequest{
			FileRef:     "easynet:///r/example/resource/alice.files/report.txt",
			OwnerURA:    "easynet:///r/example/agent/alice.sdk",
			ContentType: "text/plain",
			SizeBytes:   &size,
			ContentHash: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
			Metadata:    map[string]any{"route": "upload"},
		},
	}
}

func wrapperTerminalStartRequest() WrapperTerminalStartRequest {
	return WrapperTerminalStartRequest{
		WrapperCarrierBase: wrapperBaseForTest(),
		WrapperTerminalSessionRequest: WrapperTerminalSessionRequest{
			SessionID: "term-1",
			OwnerURA:  "easynet:///r/example/agent/alice.sdk",
			State:     "starting",
		},
		Command: []string{"bash", "-lc"},
	}
}

func wrapperRemoteDesktopStartRequest() WrapperRemoteDesktopStartRequest {
	return WrapperRemoteDesktopStartRequest{
		WrapperCarrierBase: wrapperBaseForTest(),
		WrapperRemoteDesktopSessionRequest: WrapperRemoteDesktopSessionRequest{
			SessionID:  "rdp-1",
			OwnerURA:   "easynet:///r/example/agent/alice.sdk",
			State:      "starting",
			DisplayRef: "display-main",
		},
		Display: "main",
	}
}

func wrapperBrowserStartRequest() WrapperBrowserStartRequest {
	return WrapperBrowserStartRequest{
		WrapperCarrierBase: wrapperBaseForTest(),
		WrapperBrowserSessionRequest: WrapperBrowserSessionRequest{
			SessionID:  "browser-1",
			OwnerURA:   "easynet:///r/example/agent/alice.sdk",
			State:      "starting",
			BrowserRef: "browser-main",
		},
		URL: "https://example.com",
	}
}

func wrapperMediaStartRequest() WrapperMediaStartRequest {
	return WrapperMediaStartRequest{
		WrapperCarrierBase: wrapperBaseForTest(),
		WrapperMediaSessionRequest: WrapperMediaSessionRequest{
			SessionID: "media-1",
			OwnerURA:  "easynet:///r/example/agent/alice.sdk",
			State:     "starting",
			MediaKind: "voice",
			StreamRef: "stream-voice-1",
		},
		Codec: "opus",
	}
}

func TestWrapperBuildsExecutionInvocations(t *testing.T) {
	transport := &memoryWrapperTransport{
		fileInvocation:    wrapperFileInvocationJSON,
		termInvocation:    wrapperTerminalInvocationJSON,
		rdpInvocation:     wrapperRemoteDesktopInvocationJSON,
		browserInvocation: wrapperBrowserInvocationJSON,
		mediaInvocation:   wrapperMediaInvocationJSON,
	}
	client, err := NewWrapperClientWithTransport(transport)
	if err != nil {
		t.Fatal(err)
	}

	fileDraft, err := client.BuildFileTransferInvocation(context.Background(), wrapperFileTransferRequest())
	if err != nil {
		t.Fatal(err)
	}
	if fileDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.wrapper.file.transfer@1.0.0" {
		t.Fatalf("file descriptor = %q", fileDraft.DescriptorRef())
	}
	if transport.seen["build_file"]["wrapper_kind"] != "file" || transport.seen["build_file"]["operation"] != "transfer" {
		t.Fatalf("unexpected file request: %#v", transport.seen["build_file"])
	}
	if metadata := transport.seen["build_file"]["metadata"].(map[string]any); metadata["route"] != "upload" || metadata["request_id"] != "wrapper-test" {
		t.Fatalf("metadata not merged: %#v", metadata)
	}

	terminalDraft, err := client.BuildTerminalSessionInvocation(context.Background(), wrapperTerminalStartRequest())
	if err != nil {
		t.Fatal(err)
	}
	if terminalDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.wrapper.terminal.start@1.0.0" {
		t.Fatalf("terminal descriptor = %q", terminalDraft.DescriptorRef())
	}

	remoteDraft, err := client.BuildRemoteDesktopSessionInvocation(context.Background(), wrapperRemoteDesktopStartRequest())
	if err != nil {
		t.Fatal(err)
	}
	if remoteDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.wrapper.remote_desktop.start@1.0.0" {
		t.Fatalf("remote desktop descriptor = %q", remoteDraft.DescriptorRef())
	}

	browserDraft, err := client.BuildBrowserSessionInvocation(context.Background(), wrapperBrowserStartRequest())
	if err != nil {
		t.Fatal(err)
	}
	if browserDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.wrapper.browser.start@1.0.0" {
		t.Fatalf("browser descriptor = %q", browserDraft.DescriptorRef())
	}

	mediaDraft, err := client.BuildMediaSessionInvocation(context.Background(), wrapperMediaStartRequest())
	if err != nil {
		t.Fatal(err)
	}
	if mediaDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.wrapper.media.start@1.0.0" {
		t.Fatalf("media descriptor = %q", mediaDraft.DescriptorRef())
	}
}

func TestWrapperExecutesTransportBackedHelpers(t *testing.T) {
	client, err := NewWrapperClientWithTransport(&memoryWrapperTransport{})
	if err != nil {
		t.Fatal(err)
	}
	file, err := client.TransferFile(context.Background(), wrapperFileTransferRequest())
	if err != nil {
		t.Fatal(err)
	}
	if file.Kind != "file_record" || file.FileRef == "" {
		t.Fatalf("unexpected file transfer result: %#v", file)
	}
	terminal, err := client.StartTerminalSession(context.Background(), wrapperTerminalStartRequest())
	if err != nil {
		t.Fatal(err)
	}
	if terminal.Kind != "terminal_session" || terminal.State != "active" {
		t.Fatalf("unexpected terminal result: %#v", terminal)
	}
	remote, err := client.StartRemoteDesktopSession(context.Background(), wrapperRemoteDesktopStartRequest())
	if err != nil {
		t.Fatal(err)
	}
	if remote.Kind != "remote_desktop_session" || remote.DisplayRef == nil {
		t.Fatalf("unexpected remote desktop result: %#v", remote)
	}
	browser, err := client.StartBrowserSession(context.Background(), wrapperBrowserStartRequest())
	if err != nil {
		t.Fatal(err)
	}
	if browser.Kind != "browser_session" || browser.BrowserRef == nil {
		t.Fatalf("unexpected browser result: %#v", browser)
	}
	media, err := client.StartMediaSession(context.Background(), wrapperMediaStartRequest())
	if err != nil {
		t.Fatal(err)
	}
	if media.Kind != "media_session" || media.MediaKind != "voice" {
		t.Fatalf("unexpected media result: %#v", media)
	}
}

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
	if _, err := client.TransferFile(context.Background(), wrapperFileTransferRequest()); err == nil {
		t.Fatal("expected missing wrapper transport rejection")
	}
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

const wrapperFileInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.wrapper.file.transfer@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"wrapper_kind": "file", "operation": "transfer"},
  "content_type": "application/json",
  "metadata": {"request_id": "wrapper-test", "profile": "wrappers", "system_ability": "wrapper.file.transfer", "carrier_owner": "daemon_sdk"}
}`

const wrapperTerminalInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.wrapper.terminal.start@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"wrapper_kind": "terminal", "session_id": "term-1"},
  "content_type": "application/json",
  "metadata": {"request_id": "wrapper-test", "profile": "wrappers", "system_ability": "wrapper.terminal.start", "carrier_owner": "daemon_sdk"}
}`

const wrapperRemoteDesktopInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.wrapper.remote_desktop.start@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"wrapper_kind": "remote_desktop", "session_id": "rdp-1"},
  "content_type": "application/json",
  "metadata": {"request_id": "wrapper-test", "profile": "wrappers", "system_ability": "wrapper.remote_desktop.start", "carrier_owner": "daemon_sdk"}
}`

const wrapperBrowserInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.wrapper.browser.start@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"wrapper_kind": "browser", "session_id": "browser-1"},
  "content_type": "application/json",
  "metadata": {"request_id": "wrapper-test", "profile": "wrappers", "system_ability": "wrapper.browser.start", "carrier_owner": "daemon_sdk"}
}`

const wrapperMediaInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.wrapper.media.start@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"wrapper_kind": "media", "session_id": "media-1"},
  "content_type": "application/json",
  "metadata": {"request_id": "wrapper-test", "profile": "wrappers", "system_ability": "wrapper.media.start", "carrier_owner": "daemon_sdk"}
}`

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
