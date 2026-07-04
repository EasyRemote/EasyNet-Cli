package easynet

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
)

const wrappersProfile = "wrappers"

type WrapperKind string

const (
	WrapperKindFile          WrapperKind = "file"
	WrapperKindTerminal      WrapperKind = "terminal"
	WrapperKindRemoteDesktop WrapperKind = "remote_desktop"
	WrapperKindBrowser       WrapperKind = "browser"
	WrapperKindMedia         WrapperKind = "media"
)

// WrapperCarrierBase is the complete carrier context shared by wrapper execution helpers.
type WrapperCarrierBase struct {
	CallerURA         string         `json:"caller_ura"`
	CalleeURA         string         `json:"callee_ura"`
	SubjectURA        string         `json:"subject_ura"`
	DescriptorVersion string         `json:"descriptor_version"`
	NonceBase64       string         `json:"nonce_base64"`
	CausalContext     map[string]any `json:"causal_context"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

type WrapperFileRecord struct {
	Profile     string         `json:"profile"`
	Kind        string         `json:"kind"`
	FileRef     string         `json:"file_ref"`
	OwnerURA    string         `json:"owner_ura"`
	ContentType string         `json:"content_type"`
	SizeBytes   *int64         `json:"size_bytes"`
	ContentHash *string        `json:"content_hash"`
	Metadata    map[string]any `json:"metadata"`
}

type WrapperFileRecordRequest struct {
	FileRef     string         `json:"file_ref"`
	OwnerURA    string         `json:"owner_ura"`
	ContentType string         `json:"content_type"`
	SizeBytes   *int64         `json:"size_bytes,omitempty"`
	ContentHash string         `json:"content_hash,omitempty"`
	Metadata    map[string]any `json:"metadata,omitempty"`
}

type WrapperFileTransferRequest struct {
	WrapperCarrierBase
	WrapperFileRecordRequest
	Operation   string `json:"operation,omitempty"`
	AbilityName string `json:"ability_name,omitempty"`
	Filename    string `json:"filename,omitempty"`
	BytesBase64 string `json:"bytes_b64,omitempty"`
}

type WrapperTerminalSessionRecord struct {
	Profile     string         `json:"profile"`
	Kind        string         `json:"kind"`
	SessionID   string         `json:"session_id"`
	OwnerURA    string         `json:"owner_ura"`
	State       string         `json:"state"`
	TerminalRef *string        `json:"terminal_ref"`
	Metadata    map[string]any `json:"metadata"`
}

type WrapperTerminalSessionRequest struct {
	SessionID   string         `json:"session_id"`
	OwnerURA    string         `json:"owner_ura"`
	State       string         `json:"state"`
	TerminalRef string         `json:"terminal_ref,omitempty"`
	Metadata    map[string]any `json:"metadata,omitempty"`
}

type WrapperTerminalStartRequest struct {
	WrapperCarrierBase
	WrapperTerminalSessionRequest
	Command []string `json:"command,omitempty"`
	Cwd     string   `json:"cwd,omitempty"`
}

type WrapperRemoteDesktopSessionRecord struct {
	Profile    string         `json:"profile"`
	Kind       string         `json:"kind"`
	SessionID  string         `json:"session_id"`
	OwnerURA   string         `json:"owner_ura"`
	State      string         `json:"state"`
	DisplayRef *string        `json:"display_ref"`
	Metadata   map[string]any `json:"metadata"`
}

type WrapperRemoteDesktopSessionRequest struct {
	SessionID  string         `json:"session_id"`
	OwnerURA   string         `json:"owner_ura"`
	State      string         `json:"state"`
	DisplayRef string         `json:"display_ref,omitempty"`
	Metadata   map[string]any `json:"metadata,omitempty"`
}

type WrapperRemoteDesktopStartRequest struct {
	WrapperCarrierBase
	WrapperRemoteDesktopSessionRequest
	Display string `json:"display,omitempty"`
}

type WrapperBrowserSessionRecord struct {
	Profile    string         `json:"profile"`
	Kind       string         `json:"kind"`
	SessionID  string         `json:"session_id"`
	OwnerURA   string         `json:"owner_ura"`
	State      string         `json:"state"`
	BrowserRef *string        `json:"browser_ref"`
	Metadata   map[string]any `json:"metadata"`
}

type WrapperBrowserSessionRequest struct {
	SessionID  string         `json:"session_id"`
	OwnerURA   string         `json:"owner_ura"`
	State      string         `json:"state"`
	BrowserRef string         `json:"browser_ref,omitempty"`
	Metadata   map[string]any `json:"metadata,omitempty"`
}

type WrapperBrowserStartRequest struct {
	WrapperCarrierBase
	WrapperBrowserSessionRequest
	URL string `json:"url,omitempty"`
}

type WrapperMediaSessionRecord struct {
	Profile   string         `json:"profile"`
	Kind      string         `json:"kind"`
	SessionID string         `json:"session_id"`
	OwnerURA  string         `json:"owner_ura"`
	State     string         `json:"state"`
	MediaKind string         `json:"media_kind"`
	StreamRef *string        `json:"stream_ref"`
	Metadata  map[string]any `json:"metadata"`
}

type WrapperMediaSessionRequest struct {
	SessionID string         `json:"session_id"`
	OwnerURA  string         `json:"owner_ura"`
	State     string         `json:"state"`
	MediaKind string         `json:"media_kind"`
	StreamRef string         `json:"stream_ref,omitempty"`
	Metadata  map[string]any `json:"metadata,omitempty"`
}

type WrapperMediaStartRequest struct {
	WrapperCarrierBase
	WrapperMediaSessionRequest
	Codec string `json:"codec,omitempty"`
}

type FileRecord = WrapperFileRecord
type TerminalSessionRecord = WrapperTerminalSessionRecord
type RemoteDesktopSessionRecord = WrapperRemoteDesktopSessionRecord
type BrowserSessionRecord = WrapperBrowserSessionRecord
type MediaSessionRecord = WrapperMediaSessionRecord

// WrapperTransport supplies daemon wrapper operations behind the facade.
type WrapperTransport interface {
	BuildFileTransferInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildTerminalSessionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildRemoteDesktopSessionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildBrowserSessionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildMediaSessionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	TransferFile(ctx context.Context, requestJSON []byte) ([]byte, error)
	StartTerminalSession(ctx context.Context, requestJSON []byte) ([]byte, error)
	StartRemoteDesktopSession(ctx context.Context, requestJSON []byte) ([]byte, error)
	StartBrowserSession(ctx context.Context, requestJSON []byte) ([]byte, error)
	StartMediaSession(ctx context.Context, requestJSON []byte) ([]byte, error)
}

// WrapperTransportFunc adapts functions into a WrapperTransport.
type WrapperTransportFunc struct {
	BuildFileTransferInvocationFunc         func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildTerminalSessionInvocationFunc      func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildRemoteDesktopSessionInvocationFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildBrowserSessionInvocationFunc       func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildMediaSessionInvocationFunc         func(ctx context.Context, requestJSON []byte) ([]byte, error)
	TransferFileFunc                        func(ctx context.Context, requestJSON []byte) ([]byte, error)
	StartTerminalSessionFunc                func(ctx context.Context, requestJSON []byte) ([]byte, error)
	StartRemoteDesktopSessionFunc           func(ctx context.Context, requestJSON []byte) ([]byte, error)
	StartBrowserSessionFunc                 func(ctx context.Context, requestJSON []byte) ([]byte, error)
	StartMediaSessionFunc                   func(ctx context.Context, requestJSON []byte) ([]byte, error)
}

func (f WrapperTransportFunc) BuildFileTransferInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildFileTransferInvocationFunc == nil {
		return nil, invalidProfileClient(wrappersProfile, "wrapper file-transfer invocation transport function is required")
	}
	return f.BuildFileTransferInvocationFunc(ctx, requestJSON)
}

func (f WrapperTransportFunc) BuildTerminalSessionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildTerminalSessionInvocationFunc == nil {
		return nil, invalidProfileClient(wrappersProfile, "wrapper terminal-session invocation transport function is required")
	}
	return f.BuildTerminalSessionInvocationFunc(ctx, requestJSON)
}

func (f WrapperTransportFunc) BuildRemoteDesktopSessionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildRemoteDesktopSessionInvocationFunc == nil {
		return nil, invalidProfileClient(wrappersProfile, "wrapper remote-desktop-session invocation transport function is required")
	}
	return f.BuildRemoteDesktopSessionInvocationFunc(ctx, requestJSON)
}

func (f WrapperTransportFunc) BuildBrowserSessionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildBrowserSessionInvocationFunc == nil {
		return nil, invalidProfileClient(wrappersProfile, "wrapper browser-session invocation transport function is required")
	}
	return f.BuildBrowserSessionInvocationFunc(ctx, requestJSON)
}

func (f WrapperTransportFunc) BuildMediaSessionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildMediaSessionInvocationFunc == nil {
		return nil, invalidProfileClient(wrappersProfile, "wrapper media-session invocation transport function is required")
	}
	return f.BuildMediaSessionInvocationFunc(ctx, requestJSON)
}

func (f WrapperTransportFunc) TransferFile(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.TransferFileFunc == nil {
		return nil, invalidProfileClient(wrappersProfile, "wrapper file-transfer transport function is required")
	}
	return f.TransferFileFunc(ctx, requestJSON)
}

func (f WrapperTransportFunc) StartTerminalSession(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.StartTerminalSessionFunc == nil {
		return nil, invalidProfileClient(wrappersProfile, "wrapper terminal-session transport function is required")
	}
	return f.StartTerminalSessionFunc(ctx, requestJSON)
}

func (f WrapperTransportFunc) StartRemoteDesktopSession(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.StartRemoteDesktopSessionFunc == nil {
		return nil, invalidProfileClient(wrappersProfile, "wrapper remote-desktop-session transport function is required")
	}
	return f.StartRemoteDesktopSessionFunc(ctx, requestJSON)
}

func (f WrapperTransportFunc) StartBrowserSession(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.StartBrowserSessionFunc == nil {
		return nil, invalidProfileClient(wrappersProfile, "wrapper browser-session transport function is required")
	}
	return f.StartBrowserSessionFunc(ctx, requestJSON)
}

func (f WrapperTransportFunc) StartMediaSession(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.StartMediaSessionFunc == nil {
		return nil, invalidProfileClient(wrappersProfile, "wrapper media-session transport function is required")
	}
	return f.StartMediaSessionFunc(ctx, requestJSON)
}

// WrapperClient projects daemon/resource facts and optionally executes wrapper helpers.
type WrapperClient struct {
	transport WrapperTransport
	lifecycle profileClientLifecycle
}

func NewWrapperClient() *WrapperClient {
	return &WrapperClient{}
}

func NewWrapperClientWithTransport(transport WrapperTransport) (*WrapperClient, error) {
	if transport == nil {
		return nil, invalidProfileClient(wrappersProfile, "wrapper transport is required")
	}
	return &WrapperClient{transport: transport}, nil
}

func (c *WrapperClient) BuildFileTransferInvocation(ctx context.Context, req WrapperFileTransferRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateWrapperFileTransferRequest, func(ctx context.Context, requestJSON []byte) ([]byte, error) {
		return c.transport.BuildFileTransferInvocation(ctx, requestJSON)
	}, "wrapper file-transfer invocation failed")
}

func (c *WrapperClient) BuildTerminalSessionInvocation(ctx context.Context, req WrapperTerminalStartRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateWrapperTerminalStartRequest, func(ctx context.Context, requestJSON []byte) ([]byte, error) {
		return c.transport.BuildTerminalSessionInvocation(ctx, requestJSON)
	}, "wrapper terminal-session invocation failed")
}

func (c *WrapperClient) BuildRemoteDesktopSessionInvocation(ctx context.Context, req WrapperRemoteDesktopStartRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateWrapperRemoteDesktopStartRequest, func(ctx context.Context, requestJSON []byte) ([]byte, error) {
		return c.transport.BuildRemoteDesktopSessionInvocation(ctx, requestJSON)
	}, "wrapper remote-desktop-session invocation failed")
}

func (c *WrapperClient) BuildBrowserSessionInvocation(ctx context.Context, req WrapperBrowserStartRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateWrapperBrowserStartRequest, func(ctx context.Context, requestJSON []byte) ([]byte, error) {
		return c.transport.BuildBrowserSessionInvocation(ctx, requestJSON)
	}, "wrapper browser-session invocation failed")
}

func (c *WrapperClient) BuildMediaSessionInvocation(ctx context.Context, req WrapperMediaStartRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateWrapperMediaStartRequest, func(ctx context.Context, requestJSON []byte) ([]byte, error) {
		return c.transport.BuildMediaSessionInvocation(ctx, requestJSON)
	}, "wrapper media-session invocation failed")
}

func (c *WrapperClient) TransferFile(ctx context.Context, req WrapperFileTransferRequest) (WrapperFileRecord, error) {
	raw, err := c.execute(ctx, req, validateWrapperFileTransferRequest, func(ctx context.Context, requestJSON []byte) ([]byte, error) {
		return c.transport.TransferFile(ctx, requestJSON)
	}, "wrapper file transfer failed")
	if err != nil {
		return WrapperFileRecord{}, err
	}
	return NewWrapperFileRecordFromJSON(raw)
}

func (c *WrapperClient) StartTerminalSession(ctx context.Context, req WrapperTerminalStartRequest) (WrapperTerminalSessionRecord, error) {
	raw, err := c.execute(ctx, req, validateWrapperTerminalStartRequest, func(ctx context.Context, requestJSON []byte) ([]byte, error) {
		return c.transport.StartTerminalSession(ctx, requestJSON)
	}, "wrapper terminal session failed")
	if err != nil {
		return WrapperTerminalSessionRecord{}, err
	}
	return NewWrapperTerminalSessionRecordFromJSON(raw)
}

func (c *WrapperClient) StartRemoteDesktopSession(ctx context.Context, req WrapperRemoteDesktopStartRequest) (WrapperRemoteDesktopSessionRecord, error) {
	raw, err := c.execute(ctx, req, validateWrapperRemoteDesktopStartRequest, func(ctx context.Context, requestJSON []byte) ([]byte, error) {
		return c.transport.StartRemoteDesktopSession(ctx, requestJSON)
	}, "wrapper remote desktop session failed")
	if err != nil {
		return WrapperRemoteDesktopSessionRecord{}, err
	}
	return NewWrapperRemoteDesktopSessionRecordFromJSON(raw)
}

func (c *WrapperClient) StartBrowserSession(ctx context.Context, req WrapperBrowserStartRequest) (WrapperBrowserSessionRecord, error) {
	raw, err := c.execute(ctx, req, validateWrapperBrowserStartRequest, func(ctx context.Context, requestJSON []byte) ([]byte, error) {
		return c.transport.StartBrowserSession(ctx, requestJSON)
	}, "wrapper browser session failed")
	if err != nil {
		return WrapperBrowserSessionRecord{}, err
	}
	return NewWrapperBrowserSessionRecordFromJSON(raw)
}

func (c *WrapperClient) StartMediaSession(ctx context.Context, req WrapperMediaStartRequest) (WrapperMediaSessionRecord, error) {
	raw, err := c.execute(ctx, req, validateWrapperMediaStartRequest, func(ctx context.Context, requestJSON []byte) ([]byte, error) {
		return c.transport.StartMediaSession(ctx, requestJSON)
	}, "wrapper media session failed")
	if err != nil {
		return WrapperMediaSessionRecord{}, err
	}
	return NewWrapperMediaSessionRecordFromJSON(raw)
}

func (c *WrapperClient) ProjectFileRecord(req WrapperFileRecordRequest) (WrapperFileRecord, error) {
	if c == nil {
		return WrapperFileRecord{}, invalidProfileClient(wrappersProfile, "wrapper client is not initialized")
	}
	if err := validateWrapperFileRecordRequest(req); err != nil {
		return WrapperFileRecord{}, err
	}
	record := WrapperFileRecord{
		Profile:     wrappersProfile,
		Kind:        "file_record",
		FileRef:     req.FileRef,
		OwnerURA:    req.OwnerURA,
		ContentType: req.ContentType,
		SizeBytes:   req.SizeBytes,
		Metadata:    wrapperMetadata(req.Metadata, "wrappers.file_record"),
	}
	if req.ContentHash != "" {
		record.ContentHash = &req.ContentHash
	}
	if err := validateWrapperFileRecord(record); err != nil {
		return WrapperFileRecord{}, err
	}
	return record, nil
}

func (c *WrapperClient) buildInvocation(ctx context.Context, req any, validate func(any) error, fn func(context.Context, []byte) ([]byte, error), label string) (InvocationDraft, error) {
	raw, err := c.execute(ctx, req, validate, fn, label)
	if err != nil {
		return InvocationDraft{}, err
	}
	return NewInvocationDraftFromJSON(raw)
}

func (c *WrapperClient) execute(ctx context.Context, req any, validate func(any) error, fn func(context.Context, []byte) ([]byte, error), label string) ([]byte, error) {
	if err := c.requireTransportReady(ctx); err != nil {
		return nil, err
	}
	if err := validate(req); err != nil {
		return nil, err
	}
	requestJSON, err := marshalWrapperExecutionRequest(req)
	if err != nil {
		return nil, err
	}
	raw, err := fn(ctx, requestJSON)
	if err != nil {
		return nil, wrapWrapperTransportError(label, err)
	}
	return raw, nil
}

func (c *WrapperClient) requireTransportReady(ctx context.Context) error {
	if c == nil {
		return invalidProfileClient(wrappersProfile, "wrapper client is not initialized")
	}
	if c.transport == nil {
		return invalidProfileClient(wrappersProfile, "wrapper transport is required")
	}
	return c.lifecycle.RequireOpen(ctx, "wrapper")
}

func (c *WrapperClient) Close(ctx context.Context) error {
	if c == nil {
		return invalidProfileClient(wrappersProfile, "wrapper client is not initialized")
	}
	if c.transport == nil {
		return invalidProfileClient(wrappersProfile, "wrapper transport is required")
	}
	return c.lifecycle.Close(ctx, c.transport, "wrapper")
}

func (c *WrapperClient) ProjectTerminalSession(req WrapperTerminalSessionRequest) (WrapperTerminalSessionRecord, error) {
	if c == nil {
		return WrapperTerminalSessionRecord{}, invalidProfileClient(wrappersProfile, "wrapper client is not initialized")
	}
	if err := validateWrapperSessionFacts(req.SessionID, req.OwnerURA, req.State); err != nil {
		return WrapperTerminalSessionRecord{}, err
	}
	record := WrapperTerminalSessionRecord{
		Profile:     wrappersProfile,
		Kind:        "terminal_session",
		SessionID:   req.SessionID,
		OwnerURA:    req.OwnerURA,
		State:       req.State,
		TerminalRef: optionalWrapperString(req.TerminalRef),
		Metadata:    wrapperMetadata(req.Metadata, "wrappers.terminal_session"),
	}
	if err := validateWrapperTerminalSessionRecord(record); err != nil {
		return WrapperTerminalSessionRecord{}, err
	}
	return record, nil
}

func (c *WrapperClient) ProjectRemoteDesktopSession(req WrapperRemoteDesktopSessionRequest) (WrapperRemoteDesktopSessionRecord, error) {
	if c == nil {
		return WrapperRemoteDesktopSessionRecord{}, invalidProfileClient(wrappersProfile, "wrapper client is not initialized")
	}
	if err := validateWrapperSessionFacts(req.SessionID, req.OwnerURA, req.State); err != nil {
		return WrapperRemoteDesktopSessionRecord{}, err
	}
	record := WrapperRemoteDesktopSessionRecord{
		Profile:    wrappersProfile,
		Kind:       "remote_desktop_session",
		SessionID:  req.SessionID,
		OwnerURA:   req.OwnerURA,
		State:      req.State,
		DisplayRef: optionalWrapperString(req.DisplayRef),
		Metadata:   wrapperMetadata(req.Metadata, "wrappers.remote_desktop_session"),
	}
	if err := validateWrapperRemoteDesktopSessionRecord(record); err != nil {
		return WrapperRemoteDesktopSessionRecord{}, err
	}
	return record, nil
}

func (c *WrapperClient) ProjectBrowserSession(req WrapperBrowserSessionRequest) (WrapperBrowserSessionRecord, error) {
	if c == nil {
		return WrapperBrowserSessionRecord{}, invalidProfileClient(wrappersProfile, "wrapper client is not initialized")
	}
	if err := validateWrapperSessionFacts(req.SessionID, req.OwnerURA, req.State); err != nil {
		return WrapperBrowserSessionRecord{}, err
	}
	record := WrapperBrowserSessionRecord{
		Profile:    wrappersProfile,
		Kind:       "browser_session",
		SessionID:  req.SessionID,
		OwnerURA:   req.OwnerURA,
		State:      req.State,
		BrowserRef: optionalWrapperString(req.BrowserRef),
		Metadata:   wrapperMetadata(req.Metadata, "wrappers.browser_session"),
	}
	if err := validateWrapperBrowserSessionRecord(record); err != nil {
		return WrapperBrowserSessionRecord{}, err
	}
	return record, nil
}

func (c *WrapperClient) ProjectMediaSession(req WrapperMediaSessionRequest) (WrapperMediaSessionRecord, error) {
	if c == nil {
		return WrapperMediaSessionRecord{}, invalidProfileClient(wrappersProfile, "wrapper client is not initialized")
	}
	if err := validateWrapperSessionFacts(req.SessionID, req.OwnerURA, req.State); err != nil {
		return WrapperMediaSessionRecord{}, err
	}
	if req.MediaKind == "" {
		return WrapperMediaSessionRecord{}, invalidProfilePayload(wrappersProfile, "wrapper media_kind is required", nil)
	}
	record := WrapperMediaSessionRecord{
		Profile:   wrappersProfile,
		Kind:      "media_session",
		SessionID: req.SessionID,
		OwnerURA:  req.OwnerURA,
		State:     req.State,
		MediaKind: req.MediaKind,
		StreamRef: optionalWrapperString(req.StreamRef),
		Metadata:  wrapperMetadata(req.Metadata, "wrappers.media_session"),
	}
	if err := validateWrapperMediaSessionRecord(record); err != nil {
		return WrapperMediaSessionRecord{}, err
	}
	return record, nil
}

func NewWrapperFileRecordFromJSON(raw []byte) (WrapperFileRecord, error) {
	var record WrapperFileRecord
	if err := json.Unmarshal(raw, &record); err != nil {
		return WrapperFileRecord{}, invalidProfilePayload(wrappersProfile, fmt.Sprintf("decode wrapper file record JSON: %v", err), err)
	}
	if err := validateWrapperFileRecord(record); err != nil {
		return WrapperFileRecord{}, err
	}
	return record, nil
}

func NewWrapperTerminalSessionRecordFromJSON(raw []byte) (WrapperTerminalSessionRecord, error) {
	var record WrapperTerminalSessionRecord
	if err := json.Unmarshal(raw, &record); err != nil {
		return WrapperTerminalSessionRecord{}, invalidProfilePayload(wrappersProfile, fmt.Sprintf("decode wrapper terminal session JSON: %v", err), err)
	}
	if err := validateWrapperTerminalSessionRecord(record); err != nil {
		return WrapperTerminalSessionRecord{}, err
	}
	return record, nil
}

func NewWrapperRemoteDesktopSessionRecordFromJSON(raw []byte) (WrapperRemoteDesktopSessionRecord, error) {
	var record WrapperRemoteDesktopSessionRecord
	if err := json.Unmarshal(raw, &record); err != nil {
		return WrapperRemoteDesktopSessionRecord{}, invalidProfilePayload(wrappersProfile, fmt.Sprintf("decode wrapper remote desktop session JSON: %v", err), err)
	}
	if err := validateWrapperRemoteDesktopSessionRecord(record); err != nil {
		return WrapperRemoteDesktopSessionRecord{}, err
	}
	return record, nil
}

func NewWrapperBrowserSessionRecordFromJSON(raw []byte) (WrapperBrowserSessionRecord, error) {
	var record WrapperBrowserSessionRecord
	if err := json.Unmarshal(raw, &record); err != nil {
		return WrapperBrowserSessionRecord{}, invalidProfilePayload(wrappersProfile, fmt.Sprintf("decode wrapper browser session JSON: %v", err), err)
	}
	if err := validateWrapperBrowserSessionRecord(record); err != nil {
		return WrapperBrowserSessionRecord{}, err
	}
	return record, nil
}

func NewWrapperMediaSessionRecordFromJSON(raw []byte) (WrapperMediaSessionRecord, error) {
	var record WrapperMediaSessionRecord
	if err := json.Unmarshal(raw, &record); err != nil {
		return WrapperMediaSessionRecord{}, invalidProfilePayload(wrappersProfile, fmt.Sprintf("decode wrapper media session JSON: %v", err), err)
	}
	if err := validateWrapperMediaSessionRecord(record); err != nil {
		return WrapperMediaSessionRecord{}, err
	}
	return record, nil
}

func marshalWrapperExecutionRequest(req any) ([]byte, error) {
	payload, err := wrapperExecutionPayload(req)
	if err != nil {
		return nil, err
	}
	raw, err := json.Marshal(payload)
	if err != nil {
		return nil, invalidProfilePayload(wrappersProfile, fmt.Sprintf("encode wrapper execution request: %v", err), err)
	}
	return raw, nil
}

func wrapperExecutionPayload(req any) (map[string]any, error) {
	switch value := req.(type) {
	case WrapperFileTransferRequest:
		payload := wrapperCarrierMap(value.WrapperCarrierBase)
		payload["wrapper_kind"] = string(WrapperKindFile)
		payload["operation"] = firstNonEmptyWrapper(value.Operation, "transfer")
		putWrapperString(payload, "ability_name", value.AbilityName)
		putWrapperString(payload, "filename", value.Filename)
		putWrapperString(payload, "bytes_b64", value.BytesBase64)
		putWrapperFileRecordRequest(payload, value.WrapperFileRecordRequest)
		return payload, nil
	case WrapperTerminalStartRequest:
		payload := wrapperCarrierMap(value.WrapperCarrierBase)
		payload["wrapper_kind"] = string(WrapperKindTerminal)
		putWrapperTerminalSessionRequest(payload, value.WrapperTerminalSessionRequest)
		if len(value.Command) > 0 {
			payload["command"] = value.Command
		}
		putWrapperString(payload, "cwd", value.Cwd)
		return payload, nil
	case WrapperRemoteDesktopStartRequest:
		payload := wrapperCarrierMap(value.WrapperCarrierBase)
		payload["wrapper_kind"] = string(WrapperKindRemoteDesktop)
		putWrapperRemoteDesktopSessionRequest(payload, value.WrapperRemoteDesktopSessionRequest)
		putWrapperString(payload, "display", value.Display)
		return payload, nil
	case WrapperBrowserStartRequest:
		payload := wrapperCarrierMap(value.WrapperCarrierBase)
		payload["wrapper_kind"] = string(WrapperKindBrowser)
		putWrapperBrowserSessionRequest(payload, value.WrapperBrowserSessionRequest)
		putWrapperString(payload, "url", value.URL)
		return payload, nil
	case WrapperMediaStartRequest:
		payload := wrapperCarrierMap(value.WrapperCarrierBase)
		payload["wrapper_kind"] = string(WrapperKindMedia)
		putWrapperMediaSessionRequest(payload, value.WrapperMediaSessionRequest)
		putWrapperString(payload, "codec", value.Codec)
		return payload, nil
	default:
		return nil, invalidProfilePayload(wrappersProfile, "unsupported wrapper execution request", nil)
	}
}

func wrapperCarrierMap(base WrapperCarrierBase) map[string]any {
	value := map[string]any{
		"caller_ura":         base.CallerURA,
		"callee_ura":         base.CalleeURA,
		"subject_ura":        base.SubjectURA,
		"descriptor_version": base.DescriptorVersion,
		"nonce_base64":       base.NonceBase64,
		"causal_context":     base.CausalContext,
	}
	if base.Metadata != nil {
		value["metadata"] = base.Metadata
	}
	return value
}

func putWrapperFileRecordRequest(payload map[string]any, req WrapperFileRecordRequest) {
	putWrapperString(payload, "file_ref", req.FileRef)
	payload["owner_ura"] = req.OwnerURA
	payload["content_type"] = req.ContentType
	if req.SizeBytes != nil {
		payload["size_bytes"] = *req.SizeBytes
	}
	putWrapperString(payload, "content_hash", req.ContentHash)
	mergeWrapperExecutionMetadata(payload, req.Metadata)
}

func putWrapperTerminalSessionRequest(payload map[string]any, req WrapperTerminalSessionRequest) {
	payload["session_id"] = req.SessionID
	payload["owner_ura"] = req.OwnerURA
	payload["state"] = req.State
	putWrapperString(payload, "terminal_ref", req.TerminalRef)
	mergeWrapperExecutionMetadata(payload, req.Metadata)
}

func putWrapperRemoteDesktopSessionRequest(payload map[string]any, req WrapperRemoteDesktopSessionRequest) {
	payload["session_id"] = req.SessionID
	payload["owner_ura"] = req.OwnerURA
	payload["state"] = req.State
	putWrapperString(payload, "display_ref", req.DisplayRef)
	mergeWrapperExecutionMetadata(payload, req.Metadata)
}

func putWrapperBrowserSessionRequest(payload map[string]any, req WrapperBrowserSessionRequest) {
	payload["session_id"] = req.SessionID
	payload["owner_ura"] = req.OwnerURA
	payload["state"] = req.State
	putWrapperString(payload, "browser_ref", req.BrowserRef)
	mergeWrapperExecutionMetadata(payload, req.Metadata)
}

func putWrapperMediaSessionRequest(payload map[string]any, req WrapperMediaSessionRequest) {
	payload["session_id"] = req.SessionID
	payload["owner_ura"] = req.OwnerURA
	payload["state"] = req.State
	payload["media_kind"] = req.MediaKind
	putWrapperString(payload, "stream_ref", req.StreamRef)
	mergeWrapperExecutionMetadata(payload, req.Metadata)
}

func putWrapperString(payload map[string]any, key string, value string) {
	if value != "" {
		payload[key] = value
	}
}

func validateWrapperFileRecordRequest(req WrapperFileRecordRequest) error {
	if req.FileRef == "" || req.ContentType == "" {
		return invalidProfilePayload(wrappersProfile, "wrapper file_ref and content_type are required for file records", nil)
	}
	if err := validateWrapperOwnerURA(req.OwnerURA); err != nil {
		return err
	}
	if req.SizeBytes != nil && *req.SizeBytes < 0 {
		return invalidProfilePayload(wrappersProfile, "wrapper size_bytes must be non-negative", nil)
	}
	return nil
}

func validateWrapperFileTransferRequest(req any) error {
	value := req.(WrapperFileTransferRequest)
	if err := validateWrapperCarrierBase(value.WrapperCarrierBase); err != nil {
		return err
	}
	if value.Operation != "" && strings.TrimSpace(value.Operation) != value.Operation {
		return invalidProfilePayload(wrappersProfile, "wrapper operation must not contain surrounding whitespace", nil)
	}
	if value.AbilityName != "" && strings.TrimSpace(value.AbilityName) != value.AbilityName {
		return invalidProfilePayload(wrappersProfile, "wrapper ability_name must not contain surrounding whitespace", nil)
	}
	if value.Filename != "" && strings.TrimSpace(value.Filename) != value.Filename {
		return invalidProfilePayload(wrappersProfile, "wrapper filename must not contain surrounding whitespace", nil)
	}
	if value.BytesBase64 != "" {
		if value.Filename == "" {
			return invalidProfilePayload(wrappersProfile, "wrapper filename is required when bytes_b64 is present", nil)
		}
		if _, err := base64.StdEncoding.DecodeString(value.BytesBase64); err != nil {
			return invalidProfilePayload(wrappersProfile, "wrapper bytes_b64 must be valid base64", err)
		}
		if value.OwnerURA == "" || value.ContentType == "" {
			return invalidProfilePayload(wrappersProfile, "wrapper owner_ura and content_type are required for file upload", nil)
		}
		if value.SizeBytes != nil && *value.SizeBytes < 0 {
			return invalidProfilePayload(wrappersProfile, "wrapper size_bytes must be non-negative", nil)
		}
		return validateWrapperOwnerURA(value.OwnerURA)
	}
	return validateWrapperFileRecordRequest(value.WrapperFileRecordRequest)
}

func validateWrapperTerminalStartRequest(req any) error {
	value := req.(WrapperTerminalStartRequest)
	if err := validateWrapperCarrierBase(value.WrapperCarrierBase); err != nil {
		return err
	}
	if err := validateWrapperSessionFacts(value.SessionID, value.OwnerURA, value.State); err != nil {
		return err
	}
	return validateWrapperCommand(value.Command)
}

func validateWrapperRemoteDesktopStartRequest(req any) error {
	value := req.(WrapperRemoteDesktopStartRequest)
	if err := validateWrapperCarrierBase(value.WrapperCarrierBase); err != nil {
		return err
	}
	return validateWrapperSessionFacts(value.SessionID, value.OwnerURA, value.State)
}

func validateWrapperBrowserStartRequest(req any) error {
	value := req.(WrapperBrowserStartRequest)
	if err := validateWrapperCarrierBase(value.WrapperCarrierBase); err != nil {
		return err
	}
	if err := validateWrapperSessionFacts(value.SessionID, value.OwnerURA, value.State); err != nil {
		return err
	}
	if value.URL != "" && strings.TrimSpace(value.URL) != value.URL {
		return invalidProfilePayload(wrappersProfile, "wrapper url must not contain surrounding whitespace", nil)
	}
	return nil
}

func validateWrapperMediaStartRequest(req any) error {
	value := req.(WrapperMediaStartRequest)
	if err := validateWrapperCarrierBase(value.WrapperCarrierBase); err != nil {
		return err
	}
	if err := validateWrapperSessionFacts(value.SessionID, value.OwnerURA, value.State); err != nil {
		return err
	}
	if value.MediaKind == "" {
		return invalidProfilePayload(wrappersProfile, "wrapper media_kind is required", nil)
	}
	return nil
}

func validateWrapperCarrierBase(base WrapperCarrierBase) error {
	if base.CallerURA == "" || base.CalleeURA == "" || base.SubjectURA == "" ||
		base.DescriptorVersion == "" || base.NonceBase64 == "" || base.CausalContext == nil {
		return invalidProfilePayload(wrappersProfile, "complete wrapper invocation carrier is required", nil)
	}
	return nil
}

func validateWrapperCommand(command []string) error {
	for _, part := range command {
		if part == "" || strings.TrimSpace(part) != part {
			return invalidProfilePayload(wrappersProfile, "wrapper command parts must be non-empty without surrounding whitespace", nil)
		}
	}
	return nil
}

func validateWrapperFileRecord(record WrapperFileRecord) error {
	if record.Profile != wrappersProfile || record.Kind != "file_record" ||
		record.FileRef == "" || record.ContentType == "" || record.Metadata == nil {
		return invalidProfilePayload(wrappersProfile, "invalid wrapper file record projection", nil)
	}
	if err := validateWrapperOwnerURA(record.OwnerURA); err != nil {
		return err
	}
	if record.SizeBytes != nil && *record.SizeBytes < 0 {
		return invalidProfilePayload(wrappersProfile, "wrapper size_bytes must be non-negative", nil)
	}
	return nil
}

func validateWrapperTerminalSessionRecord(record WrapperTerminalSessionRecord) error {
	if record.Profile != wrappersProfile || record.Kind != "terminal_session" || record.Metadata == nil {
		return invalidProfilePayload(wrappersProfile, "invalid wrapper terminal session projection", nil)
	}
	return validateWrapperSessionFacts(record.SessionID, record.OwnerURA, record.State)
}

func validateWrapperRemoteDesktopSessionRecord(record WrapperRemoteDesktopSessionRecord) error {
	if record.Profile != wrappersProfile || record.Kind != "remote_desktop_session" || record.Metadata == nil {
		return invalidProfilePayload(wrappersProfile, "invalid wrapper remote desktop session projection", nil)
	}
	return validateWrapperSessionFacts(record.SessionID, record.OwnerURA, record.State)
}

func validateWrapperBrowserSessionRecord(record WrapperBrowserSessionRecord) error {
	if record.Profile != wrappersProfile || record.Kind != "browser_session" || record.Metadata == nil {
		return invalidProfilePayload(wrappersProfile, "invalid wrapper browser session projection", nil)
	}
	return validateWrapperSessionFacts(record.SessionID, record.OwnerURA, record.State)
}

func validateWrapperMediaSessionRecord(record WrapperMediaSessionRecord) error {
	if record.Profile != wrappersProfile || record.Kind != "media_session" || record.MediaKind == "" || record.Metadata == nil {
		return invalidProfilePayload(wrappersProfile, "invalid wrapper media session projection", nil)
	}
	return validateWrapperSessionFacts(record.SessionID, record.OwnerURA, record.State)
}

func validateWrapperSessionFacts(sessionID, ownerURA, state string) error {
	if sessionID == "" || state == "" {
		return invalidProfilePayload(wrappersProfile, "wrapper session_id and state are required", nil)
	}
	return validateWrapperOwnerURA(ownerURA)
}

func validateWrapperOwnerURA(value string) error {
	if value == "" || strings.TrimSpace(value) != value || !strings.HasPrefix(value, "easynet://") {
		return invalidProfilePayload(wrappersProfile, "wrapper owner_ura must be an EasyNet URA", nil)
	}
	return nil
}

func wrapperMetadata(metadata map[string]any, source string) map[string]any {
	out := map[string]any{}
	for key, value := range metadata {
		out[key] = value
	}
	out["profile"] = wrappersProfile
	out["source"] = source
	return out
}

func mergeWrapperExecutionMetadata(payload map[string]any, metadata map[string]any) {
	if metadata == nil {
		return
	}
	merged := map[string]any{}
	if base, ok := payload["metadata"].(map[string]any); ok {
		for key, value := range base {
			merged[key] = value
		}
	}
	for key, value := range metadata {
		merged[key] = value
	}
	payload["metadata"] = merged
}

func optionalWrapperString(value string) *string {
	if value == "" {
		return nil
	}
	return &value
}

func firstNonEmptyWrapper(values ...string) string {
	for _, value := range values {
		if value != "" {
			return value
		}
	}
	return ""
}

func wrapWrapperTransportError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return withProfileErrorDetails(sdkErr, wrappersProfile)
	}
	return transportProfileError(wrappersProfile, message, cause)
}
