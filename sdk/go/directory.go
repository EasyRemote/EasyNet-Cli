package easynet

import (
	"context"
	"fmt"
	"strings"

	axonsdk "easynet.run/axon/sdk/go/easynet"
)

const (
	DefaultDirectoryPageLimit uint32 = 50
	MaxDirectoryPageLimit     uint32 = 500
	maxDirectoryCursorLen            = 4096
)

type DirectoryEntry = axonsdk.DirectoryEntry
type DirectoryEvent = axonsdk.DirectoryEvent
type DirectoryAgentSummary = axonsdk.DirectoryAgentSummary
type DirectorySigningAuthority = axonsdk.DirectorySigningAuthority

type DirectoryResolveKind string

const (
	DirectoryResolveRoute             DirectoryResolveKind = "RESOLVE_TYPE_ROUTE"
	DirectoryResolveListing           DirectoryResolveKind = "RESOLVE_TYPE_DIRECTORY_LISTING"
	DirectoryResolveCanonicalIdentity DirectoryResolveKind = "RESOLVE_TYPE_CANONICAL_IDENTITY"
	DirectoryResolveOwner             DirectoryResolveKind = "RESOLVE_TYPE_OWNER"
)

type DirectoryResolveRequest struct {
	Call        RuntimeCallContext   `json:"call"`
	QueryURA    string               `json:"query_ura"`
	RealmHint   string               `json:"realm_hint,omitempty"`
	AbilityName string               `json:"ability_name,omitempty"`
	Kind        DirectoryResolveKind `json:"kind,omitempty"`
}

type DirectoryRecord struct {
	Kind       string         `json:"kind"`
	URA        string         `json:"ura,omitempty"`
	OwnerURA   string         `json:"owner_ura,omitempty"`
	AbilityURA string         `json:"ability_ura,omitempty"`
	RouteURA   string         `json:"route_ura,omitempty"`
	Raw        map[string]any `json:"raw"`
}

type DirectoryResolution struct {
	AnswerKind      string            `json:"answer_kind"`
	CanonicalURA    string            `json:"canonical_ura,omitempty"`
	OwnerURA        string            `json:"owner_ura,omitempty"`
	AbilityURA      string            `json:"ability_ura,omitempty"`
	RouteURA        string            `json:"route_ura,omitempty"`
	NextHop         map[string]any    `json:"next_hop,omitempty"`
	SelectedRoute   map[string]any    `json:"selected_route,omitempty"`
	RouteCandidates []map[string]any  `json:"route_candidates,omitempty"`
	Records         []DirectoryRecord `json:"records"`
	Negative        map[string]any    `json:"negative,omitempty"`
	NextCursor      string            `json:"next_cursor,omitempty"`
	ReleaseProfile  string            `json:"release_profile,omitempty"`
	Authority       map[string]any    `json:"authority,omitempty"`
	CachePolicy     map[string]any    `json:"cache_policy,omitempty"`
}

type DirectoryListRequest struct {
	Call      RuntimeCallContext `json:"call"`
	URAPrefix string             `json:"ura_prefix"`
	Limit     uint32             `json:"limit,omitempty"`
	Cursor    string             `json:"cursor,omitempty"`
}

type DirectoryPage struct {
	Records    []DirectoryRecord `json:"records"`
	NextCursor string            `json:"next_cursor,omitempty"`
	Limit      uint32            `json:"limit"`
}

type DirectoryCursor struct {
	Sequence uint64 `json:"sequence"`
	Token    string `json:"token"`
}

func NewDirectoryCursor(sequence uint64) DirectoryCursor {
	return DirectoryCursor{Sequence: sequence, Token: fmt.Sprintf("directory:%d", sequence)}
}

type DirectorySubscriptionState string

const (
	DirectorySubscriptionSnapshotting DirectorySubscriptionState = "Snapshotting"
	DirectorySubscriptionLive         DirectorySubscriptionState = "Live"
	DirectorySubscriptionClosed       DirectorySubscriptionState = "Closed"
	DirectorySubscriptionFailed       DirectorySubscriptionState = "Failed"
)

type DirectorySubscribeRequest struct {
	Call         RuntimeCallContext `json:"call"`
	ResumeCursor *DirectoryCursor   `json:"resume_cursor,omitempty"`
}

type DirectoryEventEnvelope struct {
	Event    *DirectoryEvent `json:"event,omitempty"`
	Cursor   DirectoryCursor `json:"cursor"`
	Terminal bool            `json:"terminal"`
}

// DirectoryProvider owns runtime transport and wire projection for the
// product-neutral Directory capability.
type DirectoryProvider interface {
	Resolve(context.Context, DirectoryResolveRequest) (DirectoryResolution, error)
	List(context.Context, DirectoryListRequest) (DirectoryPage, error)
	Subscribe(context.Context, DirectorySubscribeRequest) (*DirectorySubscription, error)
}

// DirectoryClient is the stable product-neutral facade. Product read models
// build on its facts but do not redefine provider lowering.
type DirectoryClient struct {
	provider DirectoryProvider
}

func NewDirectoryClient(provider DirectoryProvider) (*DirectoryClient, error) {
	if provider == nil {
		return nil, invalidDirectory("Directory provider is required", nil)
	}
	return &DirectoryClient{provider: provider}, nil
}

func (c *DirectoryClient) Resolve(ctx context.Context, request DirectoryResolveRequest) (DirectoryResolution, error) {
	if c == nil || c.provider == nil {
		return DirectoryResolution{}, invalidDirectory("Directory client is not initialized", nil)
	}
	return c.provider.Resolve(ctx, request)
}

func (c *DirectoryClient) List(ctx context.Context, request DirectoryListRequest) (DirectoryPage, error) {
	if c == nil || c.provider == nil {
		return DirectoryPage{}, invalidDirectory("Directory client is not initialized", nil)
	}
	return c.provider.List(ctx, request)
}

func (c *DirectoryClient) Subscribe(ctx context.Context, request DirectorySubscribeRequest) (*DirectorySubscription, error) {
	if c == nil || c.provider == nil {
		return nil, invalidDirectory("Directory client is not initialized", nil)
	}
	return c.provider.Subscribe(ctx, request)
}

// RuntimeDirectoryProvider lowers Directory operations through the canonical
// RuntimeAbilityClient. Cursor and resume tokens are daemon-owned opaque
// values; the SDK forwards and validates bounded progression only.
type RuntimeDirectoryProvider struct {
	ability *RuntimeAbilityClient
}

func NewRuntimeDirectoryProvider(ability *RuntimeAbilityClient) (*RuntimeDirectoryProvider, error) {
	if ability == nil {
		return nil, invalidDirectory("runtime ability client is required", nil)
	}
	return &RuntimeDirectoryProvider{ability: ability}, nil
}

func (p *RuntimeDirectoryProvider) Resolve(ctx context.Context, request DirectoryResolveRequest) (DirectoryResolution, error) {
	if p == nil || p.ability == nil {
		return DirectoryResolution{}, invalidDirectory("runtime Directory provider is not initialized", nil)
	}
	queryURA := strings.TrimSpace(request.QueryURA)
	realmHint := strings.TrimSpace(request.RealmHint)
	if queryURA == "" && realmHint == "" {
		return DirectoryResolution{}, invalidDirectory("query_ura or realm_hint is required", nil)
	}
	kind := request.Kind
	if kind == "" {
		kind = DirectoryResolveRoute
	}
	args := map[string]any{"qtype": string(kind)}
	if queryURA != "" {
		args["query_name"] = queryURA
	}
	if realmHint != "" {
		args["realm_hint"] = realmHint
	}
	if ability := strings.TrimSpace(request.AbilityName); ability != "" {
		args["ability_name"] = ability
	}
	output, err := p.ability.Invoke(ctx, request.Call, "namespace.resolve", args)
	if err != nil {
		return DirectoryResolution{}, err
	}
	return projectDirectoryResolution(output)
}

func (p *RuntimeDirectoryProvider) List(ctx context.Context, request DirectoryListRequest) (DirectoryPage, error) {
	if p == nil || p.ability == nil {
		return DirectoryPage{}, invalidDirectory("runtime Directory provider is not initialized", nil)
	}
	limit, err := directoryLimit(request.Limit)
	if err != nil {
		return DirectoryPage{}, err
	}
	cursor, err := directoryCursor(request.Cursor)
	if err != nil {
		return DirectoryPage{}, err
	}
	args := map[string]any{
		"qtype": string(DirectoryResolveListing),
		"limit": limit,
	}
	if prefix := strings.TrimSpace(request.URAPrefix); prefix != "" {
		args["query_name"] = prefix
	}
	if cursor != "" {
		args["cursor"] = cursor
	}
	output, err := p.ability.Invoke(ctx, request.Call, "namespace.resolve", args)
	if err != nil {
		return DirectoryPage{}, err
	}
	resolution, err := projectDirectoryResolution(output)
	if err != nil {
		return DirectoryPage{}, err
	}
	if resolution.AnswerKind == "RESOLVE_ANSWER_KIND_NEGATIVE" || len(resolution.Negative) != 0 {
		return DirectoryPage{}, invalidDirectory(directoryNegativeDetail(resolution), nil)
	}
	if uint32(len(resolution.Records)) > limit {
		return DirectoryPage{}, invalidDirectory("runtime Directory listing exceeds the bounded page and has no stable cursor", nil)
	}
	nextCursor, err := directoryCursor(resolution.NextCursor)
	if err != nil {
		return DirectoryPage{}, err
	}
	if nextCursor != "" && nextCursor == cursor {
		return DirectoryPage{}, invalidDirectory("runtime Directory listing returned a repeated cursor", nil)
	}
	return DirectoryPage{Records: resolution.Records, Limit: limit, NextCursor: nextCursor}, nil
}

func (p *RuntimeDirectoryProvider) Subscribe(ctx context.Context, request DirectorySubscribeRequest) (*DirectorySubscription, error) {
	if p == nil || p.ability == nil {
		return nil, invalidDirectory("runtime Directory provider is not initialized", nil)
	}
	args := map[string]any{}
	if request.ResumeCursor != nil {
		token, err := directoryCursor(request.ResumeCursor.Token)
		if err != nil {
			return nil, err
		}
		args["resume_sequence"] = request.ResumeCursor.Sequence
		if token != "" {
			args["resume_token"] = token
		}
	}
	handle, err := p.ability.OpenStream(ctx, request.Call, "federation.subscribe_directory_v2", args)
	if err != nil {
		return nil, err
	}
	return newDirectorySubscription(handle), nil
}

type DirectorySubscription struct {
	handle *StreamHandle
	state  DirectorySubscriptionState
	cursor DirectoryCursor
}

func newDirectorySubscription(handle *StreamHandle) *DirectorySubscription {
	return &DirectorySubscription{handle: handle, state: DirectorySubscriptionSnapshotting, cursor: NewDirectoryCursor(0)}
}

func (s *DirectorySubscription) State() DirectorySubscriptionState {
	if s == nil {
		return DirectorySubscriptionFailed
	}
	return s.state
}

func (s *DirectorySubscription) Cursor() DirectoryCursor {
	if s == nil {
		return DirectoryCursor{}
	}
	return s.cursor
}

func (s *DirectorySubscription) Next(ctx context.Context) (DirectoryEventEnvelope, error) {
	if s == nil || s.handle == nil {
		return DirectoryEventEnvelope{}, invalidDirectory("Directory subscription is not initialized", nil)
	}
	if s.state == DirectorySubscriptionClosed || s.state == DirectorySubscriptionFailed {
		return DirectoryEventEnvelope{}, invalidDirectory("Directory subscription is terminal", nil)
	}
	event, err := s.handle.Next(ctx)
	if err != nil {
		s.state = DirectorySubscriptionFailed
		return DirectoryEventEnvelope{}, err
	}
	s.cursor = NewDirectoryCursor(event.Sequence())
	if event.Terminal() {
		s.state = DirectorySubscriptionClosed
		return DirectoryEventEnvelope{Cursor: s.cursor, Terminal: true}, nil
	}
	projection, err := axonsdk.ParseDirectoryEvent(event.PayloadJSON())
	if err != nil {
		s.state = DirectorySubscriptionFailed
		return DirectoryEventEnvelope{}, invalidDirectory("decode Axon Directory event", err)
	}
	if err := s.transition(projection); err != nil {
		s.state = DirectorySubscriptionFailed
		return DirectoryEventEnvelope{}, err
	}
	return DirectoryEventEnvelope{Event: &projection, Cursor: s.cursor}, nil
}

func (s *DirectorySubscription) Close(ctx context.Context) error {
	if s == nil || s.handle == nil {
		return invalidDirectory("Directory subscription is not initialized", nil)
	}
	if err := s.handle.Close(ctx); err != nil {
		s.state = DirectorySubscriptionFailed
		return err
	}
	s.state = DirectorySubscriptionClosed
	return nil
}

func (s *DirectorySubscription) transition(event DirectoryEvent) error {
	switch s.state {
	case DirectorySubscriptionSnapshotting:
		if event.Type != "snapshot" {
			return invalidDirectory("Directory subscription requires snapshot as frame zero", nil)
		}
		s.state = DirectorySubscriptionLive
	case DirectorySubscriptionLive:
		if event.Type == "snapshot" {
			return invalidDirectory("Directory subscription received a second snapshot", nil)
		}
	default:
		return invalidDirectory("Directory subscription state is terminal", nil)
	}
	return nil
}

func projectDirectoryResolution(output map[string]any) (DirectoryResolution, error) {
	if answer, ok := output["answer"].(map[string]any); ok {
		output = answer
	}
	answerKind := directoryString(output, "answer_kind")
	if answerKind == "" && len(directoryMap(output["negative"])) > 0 {
		answerKind = "RESOLVE_ANSWER_KIND_NEGATIVE"
	}
	if answerKind == "" {
		return DirectoryResolution{}, invalidDirectory("Directory answer_kind is required", nil)
	}
	records := make([]DirectoryRecord, 0)
	if rawRecords, ok := output["records"].([]any); ok {
		for _, raw := range rawRecords {
			record, ok := raw.(map[string]any)
			if !ok {
				return DirectoryResolution{}, invalidDirectory("Directory record must be an object", nil)
			}
			records = append(records, projectDirectoryRecord(record))
		}
	}
	return DirectoryResolution{
		AnswerKind:      answerKind,
		CanonicalURA:    directoryString(output, "canonical_name"),
		OwnerURA:        directoryString(output, "owner_ura"),
		AbilityURA:      directoryString(output, "ability_ura"),
		RouteURA:        directoryString(output, "route_ura"),
		NextHop:         directoryMap(output["next_hop"]),
		SelectedRoute:   directoryMap(output["selected_route"]),
		RouteCandidates: directoryMapSlice(output["route_candidates"]),
		Records:         records,
		Negative:        directoryMap(output["negative"]),
		NextCursor:      directoryString(output, "next_cursor"),
		ReleaseProfile:  directoryString(output, "release_profile"),
		Authority:       directoryMap(output["authority"]),
		CachePolicy:     directoryMap(output["cache_policy"]),
	}, nil
}

func directoryMapSlice(value any) []map[string]any {
	rows, ok := value.([]any)
	if !ok {
		return nil
	}
	result := make([]map[string]any, 0, len(rows))
	for _, row := range rows {
		object, ok := row.(map[string]any)
		if !ok {
			continue
		}
		result = append(result, directoryMap(object))
	}
	return result
}

func projectDirectoryRecord(raw map[string]any) DirectoryRecord {
	copyRaw := make(map[string]any, len(raw))
	for key, value := range raw {
		copyRaw[key] = value
	}
	return DirectoryRecord{
		Kind:       directoryString(raw, "kind", "type"),
		URA:        directoryString(raw, "ura", "canonical_name"),
		OwnerURA:   directoryString(raw, "owner_ura"),
		AbilityURA: directoryString(raw, "ability_ura"),
		RouteURA:   directoryString(raw, "route_ura"),
		Raw:        copyRaw,
	}
}

func directoryLimit(limit uint32) (uint32, error) {
	if limit == 0 {
		return DefaultDirectoryPageLimit, nil
	}
	if limit > MaxDirectoryPageLimit {
		return 0, invalidDirectory("Directory limit exceeds the maximum page bound", nil)
	}
	return limit, nil
}

func directoryCursor(value string) (string, error) {
	cursor := strings.TrimSpace(value)
	if len(cursor) > maxDirectoryCursorLen {
		return "", invalidDirectory("Directory cursor exceeds the maximum bound", nil)
	}
	return cursor, nil
}

func directoryNegativeDetail(resolution DirectoryResolution) string {
	if detail, ok := resolution.Negative["detail"].(string); ok && strings.TrimSpace(detail) != "" {
		return strings.TrimSpace(detail)
	}
	if reason, ok := resolution.Negative["reason"].(string); ok && strings.TrimSpace(reason) != "" {
		return "runtime Directory listing returned a negative answer: " + strings.TrimSpace(reason)
	}
	return "runtime Directory listing returned a negative answer"
}

func directoryString(value map[string]any, keys ...string) string {
	for _, key := range keys {
		if text, ok := value[key].(string); ok && strings.TrimSpace(text) != "" {
			return strings.TrimSpace(text)
		}
	}
	return ""
}

func directoryMap(value any) map[string]any {
	if object, ok := value.(map[string]any); ok {
		copyValue := make(map[string]any, len(object))
		for key, raw := range object {
			copyValue[key] = raw
		}
		return copyValue
	}
	return nil
}

func invalidDirectory(message string, cause error) error {
	return &SDKError{
		Code:      ErrInvalidArgument,
		Stage:     "directory",
		Retry:     RetryNever,
		Retryable: false,
		Message:   message,
		Cause:     cause,
	}
}
