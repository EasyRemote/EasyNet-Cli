package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
)

const (
	DefaultDirectoryPageSize = 50
	MaxDirectoryPageSize     = 500
)

const MaxDirectorySubscriptionBufferedEvents = 1024

const directoryIdentityProfile = "directory_identity"
const directoryReadModelSource = "read_model"
const directorySubscriptionStream = "directory"

type DirectorySubscriptionState string

const (
	DirectorySubscriptionOpening    DirectorySubscriptionState = "Opening"
	DirectorySubscriptionCatchingUp DirectorySubscriptionState = "CatchingUp"
	DirectorySubscriptionLive       DirectorySubscriptionState = "Live"
	DirectorySubscriptionResuming   DirectorySubscriptionState = "Resuming"
	DirectorySubscriptionClosed     DirectorySubscriptionState = "Closed"
	DirectorySubscriptionFailed     DirectorySubscriptionState = "Failed"
)

// DirectoryQueryBase is the complete carrier context for directory read-model requests.
type DirectoryQueryBase struct {
	CallerURA         string         `json:"caller_ura"`
	CalleeURA         string         `json:"callee_ura"`
	SubjectURA        string         `json:"subject_ura"`
	DescriptorVersion string         `json:"descriptor_version"`
	NonceBase64       string         `json:"nonce_base64"`
	CausalContext     map[string]any `json:"causal_context"`
	Limit             int            `json:"limit,omitempty"`
	Cursor            string         `json:"cursor,omitempty"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

// ResolveQuery asks the daemon-owned directory resolver for a stable projection.
type ResolveQuery struct {
	DirectoryQueryBase
	QueryName   string `json:"query_name,omitempty"`
	AbilityName string `json:"ability_name,omitempty"`
	QType       string `json:"qtype,omitempty"`
	RealmHint   string `json:"realm_hint,omitempty"`
}

// DeviceQuery requests a paginated device read model page.
type DeviceQuery struct {
	DirectoryQueryBase
}

// AgentQuery requests a paginated agent read model page.
type AgentQuery struct {
	DirectoryQueryBase
}

// AbilityQuery requests a paginated ability read model page.
type AbilityQuery struct {
	DirectoryQueryBase
	Scope      string `json:"scope,omitempty"`
	OwnerURA   string `json:"owner_ura,omitempty"`
	AbilityURA string `json:"ability_ura,omitempty"`
}

// DirectorySubscriptionCursor identifies a committed directory stream position.
type DirectorySubscriptionCursor struct {
	Stream   string `json:"stream"`
	Sequence uint64 `json:"sequence"`
	Token    string `json:"token,omitempty"`
}

func NewDirectorySubscriptionCursor(sequence uint64) DirectorySubscriptionCursor {
	return DirectorySubscriptionCursor{
		Stream:   directorySubscriptionStream,
		Sequence: sequence,
		Token:    fmt.Sprintf("%s:%d", directorySubscriptionStream, sequence),
	}
}

func (c DirectorySubscriptionCursor) ResumeToken() string {
	if c.Token != "" {
		return c.Token
	}
	if c.Stream == "" {
		return ""
	}
	return fmt.Sprintf("%s:%d", c.Stream, c.Sequence)
}

// DirectorySubscriptionRequest asks the daemon for snapshot plus live directory deltas.
type DirectorySubscriptionRequest struct {
	DirectoryQueryBase
	Stream              string                       `json:"stream,omitempty"`
	Realm               string                       `json:"realm,omitempty"`
	OwnerURA            string                       `json:"owner_ura,omitempty"`
	DeviceURA           string                       `json:"device_ura,omitempty"`
	AgentURA            string                       `json:"agent_ura,omitempty"`
	AbilityURA          string                       `json:"ability_ura,omitempty"`
	ItemKind            string                       `json:"item_kind,omitempty"`
	ResumeCursor        *DirectorySubscriptionCursor `json:"resume_cursor,omitempty"`
	HeartbeatIntervalMS int                          `json:"heartbeat_interval_ms,omitempty"`
}

// DirectorySubscriptionEvent is a bounded, typed directory stream event.
type DirectorySubscriptionEvent struct {
	Profile     string                      `json:"profile"`
	Stream      string                      `json:"stream"`
	Kind        string                      `json:"kind"`
	EventID     string                      `json:"event_id"`
	Phase       string                      `json:"phase"`
	ItemKind    string                      `json:"item_kind,omitempty"`
	Item        map[string]any              `json:"item,omitempty"`
	Cursor      DirectorySubscriptionCursor `json:"cursor"`
	ResumeToken string                      `json:"resume_token"`
	Terminal    bool                        `json:"terminal"`
	Metadata    map[string]any              `json:"metadata"`
}

// DirectorySubscription is the Directory profile subscription state seam.
type DirectorySubscription struct {
	Profile     string                       `json:"profile"`
	Kind        string                       `json:"kind"`
	Stream      string                       `json:"stream"`
	State       DirectorySubscriptionState   `json:"state"`
	Cursor      DirectorySubscriptionCursor  `json:"cursor"`
	ResumeToken string                       `json:"resume_token"`
	Events      []DirectorySubscriptionEvent `json:"events"`
	DropCount   int                          `json:"drop_count"`
	Metadata    map[string]any               `json:"metadata"`
}

// DirectoryTransport supplies read-model operations behind the SDK facade.
type DirectoryTransport interface {
	BuildDirectorySubscriptionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	Resolve(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListDevices(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListAgents(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListAbilities(ctx context.Context, requestJSON []byte) ([]byte, error)
	SubscribeDirectory(ctx context.Context, requestJSON []byte) ([]byte, error)
}

// DirectoryTransportFunc adapts functions into a DirectoryTransport.
type DirectoryTransportFunc struct {
	BuildDirectorySubscriptionInvocationFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ResolveFunc                              func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListDevicesFunc                          func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListAgentsFunc                           func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListAbilitiesFunc                        func(ctx context.Context, requestJSON []byte) ([]byte, error)
	SubscribeDirectoryFunc                   func(ctx context.Context, requestJSON []byte) ([]byte, error)
}

func (f DirectoryTransportFunc) BuildDirectorySubscriptionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildDirectorySubscriptionInvocationFunc == nil {
		return nil, invalidRuntimeClient("directory subscription invocation transport function is required")
	}
	return f.BuildDirectorySubscriptionInvocationFunc(ctx, requestJSON)
}

func (f DirectoryTransportFunc) Resolve(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ResolveFunc == nil {
		return nil, invalidRuntimeClient("directory resolve transport function is required")
	}
	return f.ResolveFunc(ctx, requestJSON)
}

func (f DirectoryTransportFunc) ListDevices(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ListDevicesFunc == nil {
		return nil, invalidRuntimeClient("directory list-devices transport function is required")
	}
	return f.ListDevicesFunc(ctx, requestJSON)
}

func (f DirectoryTransportFunc) ListAgents(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ListAgentsFunc == nil {
		return nil, invalidRuntimeClient("directory list-agents transport function is required")
	}
	return f.ListAgentsFunc(ctx, requestJSON)
}

func (f DirectoryTransportFunc) ListAbilities(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ListAbilitiesFunc == nil {
		return nil, invalidRuntimeClient("directory list-abilities transport function is required")
	}
	return f.ListAbilitiesFunc(ctx, requestJSON)
}

func (f DirectoryTransportFunc) SubscribeDirectory(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.SubscribeDirectoryFunc == nil {
		return nil, invalidRuntimeClient("directory subscribe transport function is required")
	}
	return f.SubscribeDirectoryFunc(ctx, requestJSON)
}

// DirectoryClient is the Directory + Identity read-model facade.
type DirectoryClient struct {
	lifecycle profileClientLifecycle
	transport DirectoryTransport
}

func NewDirectoryClient(transport DirectoryTransport) (*DirectoryClient, error) {
	if transport == nil {
		return nil, invalidRuntimeClient("directory transport is required")
	}
	return &DirectoryClient{transport: transport}, nil
}

func (c *DirectoryClient) BuildDirectorySubscriptionInvocation(ctx context.Context, req DirectorySubscriptionRequest) (InvocationDraft, error) {
	if err := c.requireReady(ctx); err != nil {
		return InvocationDraft{}, err
	}
	requestJSON, err := marshalDirectorySubscriptionRequest(req)
	if err != nil {
		return InvocationDraft{}, err
	}
	raw, err := c.transport.BuildDirectorySubscriptionInvocation(ctx, requestJSON)
	if err != nil {
		return InvocationDraft{}, wrapDirectoryTransportError("directory subscription invocation failed", err)
	}
	return NewInvocationDraftFromJSON(raw)
}

func (c *DirectoryClient) SubscribeDirectory(ctx context.Context, req DirectorySubscriptionRequest) (DirectorySubscription, error) {
	if err := c.requireReady(ctx); err != nil {
		return DirectorySubscription{}, err
	}
	requestJSON, err := marshalDirectorySubscriptionRequest(req)
	if err != nil {
		return DirectorySubscription{}, err
	}
	raw, err := c.transport.SubscribeDirectory(ctx, requestJSON)
	if err != nil {
		return DirectorySubscription{}, wrapDirectoryTransportError("directory subscribe failed", err)
	}
	return NewDirectorySubscriptionFromJSON(raw)
}

func (c *DirectoryClient) Resolve(ctx context.Context, query ResolveQuery) (ResolvedRef, error) {
	if err := c.requireReady(ctx); err != nil {
		return ResolvedRef{}, err
	}
	if err := validateDirectoryQueryBase(query.DirectoryQueryBase, false); err != nil {
		return ResolvedRef{}, err
	}
	if query.QueryName == "" && query.RealmHint == "" {
		return ResolvedRef{}, invalidRuntimePayload("query_name or realm_hint is required", nil)
	}
	requestJSON, err := json.Marshal(query)
	if err != nil {
		return ResolvedRef{}, invalidRuntimePayload(fmt.Sprintf("encode directory resolve request: %v", err), err)
	}
	raw, err := c.transport.Resolve(ctx, requestJSON)
	if err != nil {
		return ResolvedRef{}, wrapDirectoryTransportError("directory resolve failed", err)
	}
	return NewResolvedRefFromJSON(raw)
}

func (c *DirectoryClient) ListDevices(ctx context.Context, query DeviceQuery) (DevicePage, error) {
	if err := c.requireReady(ctx); err != nil {
		return DevicePage{}, err
	}
	requestJSON, err := marshalDirectoryPageQuery(query.DirectoryQueryBase)
	if err != nil {
		return DevicePage{}, err
	}
	raw, err := c.transport.ListDevices(ctx, requestJSON)
	if err != nil {
		return DevicePage{}, wrapDirectoryTransportError("directory list devices failed", err)
	}
	return NewDevicePageFromJSON(raw)
}

func (c *DirectoryClient) ListAgents(ctx context.Context, query AgentQuery) (AgentPage, error) {
	if err := c.requireReady(ctx); err != nil {
		return AgentPage{}, err
	}
	requestJSON, err := marshalDirectoryPageQuery(query.DirectoryQueryBase)
	if err != nil {
		return AgentPage{}, err
	}
	raw, err := c.transport.ListAgents(ctx, requestJSON)
	if err != nil {
		return AgentPage{}, wrapDirectoryTransportError("directory list agents failed", err)
	}
	return NewAgentPageFromJSON(raw)
}

func (c *DirectoryClient) ListAbilities(ctx context.Context, query AbilityQuery) (AbilityPage, error) {
	if err := c.requireReady(ctx); err != nil {
		return AbilityPage{}, err
	}
	query.DirectoryQueryBase = normalizeDirectoryPageQuery(query.DirectoryQueryBase)
	if err := validateDirectoryQueryBase(query.DirectoryQueryBase, true); err != nil {
		return AbilityPage{}, err
	}
	requestJSON, err := json.Marshal(query)
	if err != nil {
		return AbilityPage{}, invalidRuntimePayload(fmt.Sprintf("encode directory ability query: %v", err), err)
	}
	raw, err := c.transport.ListAbilities(ctx, requestJSON)
	if err != nil {
		return AbilityPage{}, wrapDirectoryTransportError("directory list abilities failed", err)
	}
	return NewAbilityPageFromJSON(raw)
}

func (c *DirectoryClient) requireReady(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidRuntimeClient("directory client is not initialized")
	}
	return c.lifecycle.RequireOpen(ctx, "directory")
}

func (c *DirectoryClient) Close(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidRuntimeClient("directory client is not initialized")
	}
	return c.lifecycle.Close(ctx, c.transport, "directory")
}

// ResolvedRef is the daemon/Axon-owned directory resolution projection.
type ResolvedRef struct {
	Profile         string           `json:"profile"`
	Kind            string           `json:"kind"`
	AnswerKind      string           `json:"answer_kind"`
	QueryName       *string          `json:"query_name"`
	CanonicalName   *string          `json:"canonical_name"`
	OwnerURA        *string          `json:"owner_ura"`
	AbilityURA      *string          `json:"ability_ura"`
	RouteURA        *string          `json:"route_ura"`
	NextHop         map[string]any   `json:"next_hop"`
	SelectedRoute   map[string]any   `json:"selected_route"`
	RouteCandidates []map[string]any `json:"route_candidates"`
	Records         []map[string]any `json:"records"`
	Negative        map[string]any   `json:"negative"`
	ReleaseProfile  *string          `json:"release_profile"`
	Authority       map[string]any   `json:"authority"`
	CachePolicy     map[string]any   `json:"cache_policy"`
	Metadata        map[string]any   `json:"metadata"`
}

type DeviceRecord struct {
	Profile     string         `json:"profile"`
	Kind        string         `json:"kind"`
	NodeID      string         `json:"node_id"`
	DeviceURA   string         `json:"device_ura"`
	State       string         `json:"state"`
	Online      bool           `json:"online"`
	IsSelf      bool           `json:"is_self"`
	Paired      bool           `json:"paired"`
	TenantID    string         `json:"tenant_id"`
	HubEndpoint string         `json:"hub_endpoint"`
	ProbeStatus string         `json:"probe_status"`
	ProbeError  any            `json:"probe_error"`
	LatencyMS   int            `json:"latency_ms"`
	Abilities   []string       `json:"abilities"`
	Metadata    map[string]any `json:"metadata"`
}

type AgentRecord struct {
	Name      string         `json:"name"`
	AgentURA  *string        `json:"agent_ura"`
	OwnerURA  *string        `json:"owner_ura"`
	DeviceURA *string        `json:"device_ura"`
	State     string         `json:"state"`
	Runtime   string         `json:"runtime"`
	Model     *string        `json:"model"`
	Label     *string        `json:"label"`
	Abilities []string       `json:"abilities"`
	Metadata  map[string]any `json:"metadata"`
}

type AbilityRecord struct {
	Profile           string         `json:"profile"`
	Kind              string         `json:"kind"`
	Name              string         `json:"name"`
	AbilityURA        string         `json:"ability_ura"`
	OwnerURA          string         `json:"owner_ura"`
	DescriptorRef     *string        `json:"descriptor_ref"`
	DescriptorVersion string         `json:"descriptor_version"`
	Visibility        string         `json:"visibility"`
	Class             string         `json:"class"`
	Description       string         `json:"description"`
	Source            string         `json:"source"`
	SchemaSummary     map[string]any `json:"schema_summary"`
	Hints             map[string]any `json:"hints"`
	Metadata          map[string]any `json:"metadata"`
}

type DevicePage struct {
	Profile    string         `json:"profile"`
	Kind       string         `json:"kind"`
	ItemKind   string         `json:"item_kind"`
	Items      []DeviceRecord `json:"items"`
	NextCursor *string        `json:"next_cursor"`
	Limit      int            `json:"limit"`
	Source     string         `json:"source"`
	Metadata   map[string]any `json:"metadata"`
}

type AgentPage struct {
	Profile    string         `json:"profile"`
	Kind       string         `json:"kind"`
	ItemKind   string         `json:"item_kind"`
	Items      []AgentRecord  `json:"items"`
	NextCursor *string        `json:"next_cursor"`
	Limit      int            `json:"limit"`
	Source     string         `json:"source"`
	Metadata   map[string]any `json:"metadata"`
}

type AbilityPage struct {
	Profile    string          `json:"profile"`
	Kind       string          `json:"kind"`
	ItemKind   string          `json:"item_kind"`
	Items      []AbilityRecord `json:"items"`
	NextCursor *string         `json:"next_cursor"`
	Limit      int             `json:"limit"`
	Source     string          `json:"source"`
	Metadata   map[string]any  `json:"metadata"`
}

func NewResolvedRefFromJSON(raw []byte) (ResolvedRef, error) {
	var ref ResolvedRef
	if err := json.Unmarshal(raw, &ref); err != nil {
		return ResolvedRef{}, invalidRuntimePayload(fmt.Sprintf("decode resolved ref JSON: %v", err), err)
	}
	if ref.Profile != directoryIdentityProfile || ref.Kind != "resolved_ref" || ref.AnswerKind == "" {
		return ResolvedRef{}, invalidRuntimePayload("invalid directory resolved_ref projection", nil)
	}
	if ref.Metadata == nil {
		ref.Metadata = map[string]any{}
	}
	return ref, nil
}

func NewDevicePageFromJSON(raw []byte) (DevicePage, error) {
	var page DevicePage
	if err := json.Unmarshal(raw, &page); err != nil {
		return DevicePage{}, invalidRuntimePayload(fmt.Sprintf("decode device page JSON: %v", err), err)
	}
	if err := validateDirectoryPage(page.Profile, page.Kind, page.ItemKind, page.Source, page.Limit, "device_page", "device"); err != nil {
		return DevicePage{}, err
	}
	return page, nil
}

func NewAgentPageFromJSON(raw []byte) (AgentPage, error) {
	var page AgentPage
	if err := json.Unmarshal(raw, &page); err != nil {
		return AgentPage{}, invalidRuntimePayload(fmt.Sprintf("decode agent page JSON: %v", err), err)
	}
	if err := validateDirectoryPage(page.Profile, page.Kind, page.ItemKind, page.Source, page.Limit, "agent_page", "agent"); err != nil {
		return AgentPage{}, err
	}
	return page, nil
}

func NewAbilityPageFromJSON(raw []byte) (AbilityPage, error) {
	var page AbilityPage
	if err := json.Unmarshal(raw, &page); err != nil {
		return AbilityPage{}, invalidRuntimePayload(fmt.Sprintf("decode ability page JSON: %v", err), err)
	}
	if err := validateDirectoryPage(page.Profile, page.Kind, page.ItemKind, page.Source, page.Limit, "ability_page", "ability"); err != nil {
		return AbilityPage{}, err
	}
	return page, nil
}

func NewDirectorySubscriptionFromJSON(raw []byte) (DirectorySubscription, error) {
	var subscription DirectorySubscription
	if err := json.Unmarshal(raw, &subscription); err != nil {
		return DirectorySubscription{}, invalidRuntimePayload(fmt.Sprintf("decode directory subscription JSON: %v", err), err)
	}
	if err := validateDirectorySubscription(&subscription); err != nil {
		return DirectorySubscription{}, err
	}
	return subscription, nil
}

func marshalDirectorySubscriptionRequest(req DirectorySubscriptionRequest) ([]byte, error) {
	normalized, err := normalizeDirectorySubscriptionRequest(req)
	if err != nil {
		return nil, err
	}
	raw, err := json.Marshal(normalized)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode directory subscription request: %v", err), err)
	}
	return raw, nil
}

func marshalDirectoryPageQuery(query DirectoryQueryBase) ([]byte, error) {
	query = normalizeDirectoryPageQuery(query)
	if err := validateDirectoryQueryBase(query, true); err != nil {
		return nil, err
	}
	raw, err := json.Marshal(query)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode directory page query: %v", err), err)
	}
	return raw, nil
}

func normalizeDirectorySubscriptionRequest(req DirectorySubscriptionRequest) (DirectorySubscriptionRequest, error) {
	if req.Stream == "" {
		req.Stream = directorySubscriptionStream
	}
	if req.Stream != directorySubscriptionStream {
		return DirectorySubscriptionRequest{}, invalidRuntimePayload("directory subscription stream mismatch", nil)
	}
	if err := validateDirectoryQueryBase(req.DirectoryQueryBase, false); err != nil {
		return DirectorySubscriptionRequest{}, err
	}
	for field, value := range map[string]string{
		"realm":       req.Realm,
		"owner_ura":   req.OwnerURA,
		"device_ura":  req.DeviceURA,
		"agent_ura":   req.AgentURA,
		"ability_ura": req.AbilityURA,
		"item_kind":   req.ItemKind,
	} {
		if strings.TrimSpace(value) != value {
			return DirectorySubscriptionRequest{}, invalidRuntimePayload(field+" must not contain surrounding whitespace", nil)
		}
	}
	if req.ResumeCursor != nil {
		if err := validateDirectorySubscriptionCursor(*req.ResumeCursor); err != nil {
			return DirectorySubscriptionRequest{}, err
		}
	}
	if req.HeartbeatIntervalMS < 0 {
		return DirectorySubscriptionRequest{}, invalidRuntimePayload("heartbeat_interval_ms must be non-negative", nil)
	}
	return req, nil
}

func normalizeDirectoryPageQuery(query DirectoryQueryBase) DirectoryQueryBase {
	if query.Limit == 0 {
		query.Limit = DefaultDirectoryPageSize
	}
	return query
}

func validateDirectoryQueryBase(query DirectoryQueryBase, requireLimit bool) error {
	if query.CallerURA == "" || query.CalleeURA == "" || query.SubjectURA == "" || query.DescriptorVersion == "" || query.NonceBase64 == "" {
		return invalidRuntimePayload("caller_ura, callee_ura, subject_ura, descriptor_version, and nonce_base64 are required", nil)
	}
	if query.CausalContext == nil {
		return invalidRuntimePayload("causal_context is required", nil)
	}
	if requireLimit {
		if query.Limit < 1 || query.Limit > MaxDirectoryPageSize {
			return invalidRuntimePayload("directory page limit exceeds bounds", nil)
		}
	}
	return nil
}

func validateDirectorySubscription(subscription *DirectorySubscription) error {
	if subscription.Profile != directoryIdentityProfile || subscription.Kind != "directory_subscription" ||
		subscription.Stream != directorySubscriptionStream || subscription.Metadata == nil {
		return invalidRuntimePayload("invalid directory subscription projection", nil)
	}
	if !validDirectorySubscriptionState(subscription.State) {
		return invalidRuntimePayload("invalid directory subscription state", nil)
	}
	if err := validateDirectorySubscriptionCursor(subscription.Cursor); err != nil {
		return err
	}
	if subscription.ResumeToken == "" {
		subscription.ResumeToken = subscription.Cursor.ResumeToken()
	}
	if subscription.ResumeToken != subscription.Cursor.ResumeToken() {
		return invalidRuntimePayload("directory subscription resume token mismatch", nil)
	}
	if subscription.DropCount < 0 {
		return invalidRuntimePayload("directory subscription drop_count must be non-negative", nil)
	}
	if len(subscription.Events) > MaxDirectorySubscriptionBufferedEvents {
		return invalidRuntimePayload("directory subscription buffered events exceeds bounds", nil)
	}
	return validateDirectorySubscriptionEvents(subscription.Events)
}

func validateDirectorySubscriptionEvents(events []DirectorySubscriptionEvent) error {
	seen := map[string]struct{}{}
	snapshotComplete := false
	lastSequence := uint64(0)
	for idx := range events {
		event := &events[idx]
		if event.Profile != directoryIdentityProfile || event.Stream != directorySubscriptionStream ||
			event.Kind == "" || event.EventID == "" || event.Phase == "" || event.Metadata == nil {
			return invalidRuntimePayload("invalid directory subscription event projection", nil)
		}
		if _, ok := seen[event.EventID]; ok {
			return invalidRuntimePayload("duplicate directory subscription event id", nil)
		}
		seen[event.EventID] = struct{}{}
		if err := validateDirectorySubscriptionCursor(event.Cursor); err != nil {
			return err
		}
		if idx > 0 && event.Cursor.Sequence <= lastSequence {
			return invalidRuntimePayload("directory subscription event sequence must increase", nil)
		}
		lastSequence = event.Cursor.Sequence
		if event.ResumeToken == "" {
			event.ResumeToken = event.Cursor.ResumeToken()
		}
		if event.ResumeToken != event.Cursor.ResumeToken() {
			return invalidRuntimePayload("directory subscription event resume token mismatch", nil)
		}
		if event.Phase == "live" && !snapshotComplete {
			return invalidRuntimePayload("directory live event before snapshot_complete", nil)
		}
		if event.Phase == "snapshot_complete" {
			snapshotComplete = true
		}
	}
	return nil
}

func validateDirectorySubscriptionCursor(cursor DirectorySubscriptionCursor) error {
	if cursor.Stream != directorySubscriptionStream {
		return invalidRuntimePayload("directory subscription cursor stream mismatch", nil)
	}
	token := cursor.ResumeToken()
	if token == "" || strings.ContainsAny(token, " \t\r\n") {
		return invalidRuntimePayload("directory subscription cursor token is invalid", nil)
	}
	if token != fmt.Sprintf("%s:%d", cursor.Stream, cursor.Sequence) {
		return invalidRuntimePayload("directory subscription cursor token mismatch", nil)
	}
	return nil
}

func validDirectorySubscriptionState(state DirectorySubscriptionState) bool {
	switch state {
	case DirectorySubscriptionOpening, DirectorySubscriptionCatchingUp, DirectorySubscriptionLive,
		DirectorySubscriptionResuming, DirectorySubscriptionClosed, DirectorySubscriptionFailed:
		return true
	default:
		return false
	}
}

func validateDirectoryPage(profile string, kind string, itemKind string, source string, limit int, wantKind string, wantItem string) error {
	if profile != directoryIdentityProfile || kind != wantKind || itemKind != wantItem || source != directoryReadModelSource {
		return invalidRuntimePayload("invalid directory page projection", nil)
	}
	if limit < 1 || limit > MaxDirectoryPageSize {
		return invalidRuntimePayload("directory page limit exceeds bounds", nil)
	}
	return nil
}

func wrapDirectoryTransportError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return sdkErr
	}
	return transportRuntimeError(message, cause)
}
