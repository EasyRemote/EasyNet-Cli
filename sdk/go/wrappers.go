package easynet

import (
	"encoding/json"
	"fmt"
	"strings"
)

const wrappersProfile = "wrappers"

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

type FileRecord = WrapperFileRecord
type TerminalSessionRecord = WrapperTerminalSessionRecord
type RemoteDesktopSessionRecord = WrapperRemoteDesktopSessionRecord
type BrowserSessionRecord = WrapperBrowserSessionRecord
type MediaSessionRecord = WrapperMediaSessionRecord

// WrapperClient projects daemon/resource facts into SDK wrapper records.
type WrapperClient struct{}

func NewWrapperClient() *WrapperClient {
	return &WrapperClient{}
}

func (c *WrapperClient) ProjectFileRecord(req WrapperFileRecordRequest) (WrapperFileRecord, error) {
	if c == nil {
		return WrapperFileRecord{}, invalidRuntimeClient("wrapper client is not initialized")
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

func (c *WrapperClient) ProjectTerminalSession(req WrapperTerminalSessionRequest) (WrapperTerminalSessionRecord, error) {
	if c == nil {
		return WrapperTerminalSessionRecord{}, invalidRuntimeClient("wrapper client is not initialized")
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
		return WrapperRemoteDesktopSessionRecord{}, invalidRuntimeClient("wrapper client is not initialized")
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
		return WrapperBrowserSessionRecord{}, invalidRuntimeClient("wrapper client is not initialized")
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
		return WrapperMediaSessionRecord{}, invalidRuntimeClient("wrapper client is not initialized")
	}
	if err := validateWrapperSessionFacts(req.SessionID, req.OwnerURA, req.State); err != nil {
		return WrapperMediaSessionRecord{}, err
	}
	if req.MediaKind == "" {
		return WrapperMediaSessionRecord{}, invalidRuntimePayload("wrapper media_kind is required", nil)
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
		return WrapperFileRecord{}, invalidRuntimePayload(fmt.Sprintf("decode wrapper file record JSON: %v", err), err)
	}
	if err := validateWrapperFileRecord(record); err != nil {
		return WrapperFileRecord{}, err
	}
	return record, nil
}

func NewWrapperTerminalSessionRecordFromJSON(raw []byte) (WrapperTerminalSessionRecord, error) {
	var record WrapperTerminalSessionRecord
	if err := json.Unmarshal(raw, &record); err != nil {
		return WrapperTerminalSessionRecord{}, invalidRuntimePayload(fmt.Sprintf("decode wrapper terminal session JSON: %v", err), err)
	}
	if err := validateWrapperTerminalSessionRecord(record); err != nil {
		return WrapperTerminalSessionRecord{}, err
	}
	return record, nil
}

func NewWrapperRemoteDesktopSessionRecordFromJSON(raw []byte) (WrapperRemoteDesktopSessionRecord, error) {
	var record WrapperRemoteDesktopSessionRecord
	if err := json.Unmarshal(raw, &record); err != nil {
		return WrapperRemoteDesktopSessionRecord{}, invalidRuntimePayload(fmt.Sprintf("decode wrapper remote desktop session JSON: %v", err), err)
	}
	if err := validateWrapperRemoteDesktopSessionRecord(record); err != nil {
		return WrapperRemoteDesktopSessionRecord{}, err
	}
	return record, nil
}

func NewWrapperBrowserSessionRecordFromJSON(raw []byte) (WrapperBrowserSessionRecord, error) {
	var record WrapperBrowserSessionRecord
	if err := json.Unmarshal(raw, &record); err != nil {
		return WrapperBrowserSessionRecord{}, invalidRuntimePayload(fmt.Sprintf("decode wrapper browser session JSON: %v", err), err)
	}
	if err := validateWrapperBrowserSessionRecord(record); err != nil {
		return WrapperBrowserSessionRecord{}, err
	}
	return record, nil
}

func NewWrapperMediaSessionRecordFromJSON(raw []byte) (WrapperMediaSessionRecord, error) {
	var record WrapperMediaSessionRecord
	if err := json.Unmarshal(raw, &record); err != nil {
		return WrapperMediaSessionRecord{}, invalidRuntimePayload(fmt.Sprintf("decode wrapper media session JSON: %v", err), err)
	}
	if err := validateWrapperMediaSessionRecord(record); err != nil {
		return WrapperMediaSessionRecord{}, err
	}
	return record, nil
}

func validateWrapperFileRecordRequest(req WrapperFileRecordRequest) error {
	if req.FileRef == "" || req.ContentType == "" {
		return invalidRuntimePayload("wrapper file_ref and content_type are required", nil)
	}
	if err := validateWrapperOwnerURA(req.OwnerURA); err != nil {
		return err
	}
	if req.SizeBytes != nil && *req.SizeBytes < 0 {
		return invalidRuntimePayload("wrapper size_bytes must be non-negative", nil)
	}
	return nil
}

func validateWrapperFileRecord(record WrapperFileRecord) error {
	if record.Profile != wrappersProfile || record.Kind != "file_record" ||
		record.FileRef == "" || record.ContentType == "" || record.Metadata == nil {
		return invalidRuntimePayload("invalid wrapper file record projection", nil)
	}
	if err := validateWrapperOwnerURA(record.OwnerURA); err != nil {
		return err
	}
	if record.SizeBytes != nil && *record.SizeBytes < 0 {
		return invalidRuntimePayload("wrapper size_bytes must be non-negative", nil)
	}
	return nil
}

func validateWrapperTerminalSessionRecord(record WrapperTerminalSessionRecord) error {
	if record.Profile != wrappersProfile || record.Kind != "terminal_session" || record.Metadata == nil {
		return invalidRuntimePayload("invalid wrapper terminal session projection", nil)
	}
	return validateWrapperSessionFacts(record.SessionID, record.OwnerURA, record.State)
}

func validateWrapperRemoteDesktopSessionRecord(record WrapperRemoteDesktopSessionRecord) error {
	if record.Profile != wrappersProfile || record.Kind != "remote_desktop_session" || record.Metadata == nil {
		return invalidRuntimePayload("invalid wrapper remote desktop session projection", nil)
	}
	return validateWrapperSessionFacts(record.SessionID, record.OwnerURA, record.State)
}

func validateWrapperBrowserSessionRecord(record WrapperBrowserSessionRecord) error {
	if record.Profile != wrappersProfile || record.Kind != "browser_session" || record.Metadata == nil {
		return invalidRuntimePayload("invalid wrapper browser session projection", nil)
	}
	return validateWrapperSessionFacts(record.SessionID, record.OwnerURA, record.State)
}

func validateWrapperMediaSessionRecord(record WrapperMediaSessionRecord) error {
	if record.Profile != wrappersProfile || record.Kind != "media_session" || record.MediaKind == "" || record.Metadata == nil {
		return invalidRuntimePayload("invalid wrapper media session projection", nil)
	}
	return validateWrapperSessionFacts(record.SessionID, record.OwnerURA, record.State)
}

func validateWrapperSessionFacts(sessionID, ownerURA, state string) error {
	if sessionID == "" || state == "" {
		return invalidRuntimePayload("wrapper session_id and state are required", nil)
	}
	return validateWrapperOwnerURA(ownerURA)
}

func validateWrapperOwnerURA(value string) error {
	if value == "" || strings.TrimSpace(value) != value || !strings.HasPrefix(value, "easynet://") {
		return invalidRuntimePayload("wrapper owner_ura must be an EasyNet URA", nil)
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

func optionalWrapperString(value string) *string {
	if value == "" {
		return nil
	}
	return &value
}
