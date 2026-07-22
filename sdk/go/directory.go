package easynet

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"

	directorycore "easynet.run/cli/sdk/go/directorycore"
)

const (
	DefaultDirectoryPageLimit uint32 = 50
	MaxDirectoryPageLimit     uint32 = 500
	maxDirectoryCursorLen            = 4096
)

type DirectoryResolveKind = directorycore.ResolveKind

const (
	DirectoryResolveRoute             = directorycore.ResolveRoute
	DirectoryResolveListing           = directorycore.ResolveListing
	DirectoryResolveCanonicalIdentity = directorycore.ResolveCanonicalIdentity
	DirectoryResolveOwner             = directorycore.ResolveOwner
)

type DirectoryResolveRequest struct {
	Call             RuntimeCallContext   `json:"call"`
	QueryURA         string               `json:"query_ura"`
	RealmHint        string               `json:"realm_hint,omitempty"`
	AbilityName      string               `json:"ability_name,omitempty"`
	Kind             DirectoryResolveKind `json:"kind,omitempty"`
	IncludeAbilities *bool                `json:"include_abilities,omitempty"`
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
	Call             RuntimeCallContext `json:"call"`
	URAPrefix        string             `json:"ura_prefix"`
	Limit            uint32             `json:"limit,omitempty"`
	Cursor           string             `json:"cursor,omitempty"`
	IncludeAbilities *bool              `json:"include_abilities,omitempty"`
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

type DirectoryEvent struct {
	Type string         `json:"type"`
	Raw  map[string]any `json:"raw"`
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
	projection, err := ParseDirectoryEvent(event.PayloadJSON())
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

func ParseDirectoryEvent(raw []byte) (DirectoryEvent, error) {
	var event map[string]any
	if err := json.Unmarshal(raw, &event); err != nil {
		return DirectoryEvent{}, fmt.Errorf("directory event: decode JSON: %w", err)
	}
	eventType := directoryString(event, "type")
	if eventType == "" {
		return DirectoryEvent{}, invalidDirectory("Directory event type is required", nil)
	}
	return DirectoryEvent{Type: eventType, Raw: directoryMap(event)}, nil
}

// ProjectDirectoryResolution projects a daemon namespace.resolve output object
// into the SDK's product-neutral DirectoryResolution model.
func ProjectDirectoryResolution(output map[string]any) (DirectoryResolution, error) {
	if rawAnswer, present := output["answer"]; present && rawAnswer != nil {
		answer, ok := rawAnswer.(map[string]any)
		if !ok {
			return DirectoryResolution{}, invalidDirectory("Directory answer must be an object", nil)
		}
		output = answer
	}
	answerKind := directoryString(output, "answer_kind")
	negative, err := optionalDirectoryMap(output, "negative")
	if err != nil {
		return DirectoryResolution{}, err
	}
	if answerKind == "" && len(negative) > 0 {
		answerKind = "RESOLVE_ANSWER_KIND_NEGATIVE"
	}
	if answerKind == "" {
		return DirectoryResolution{}, invalidDirectory("Directory answer_kind is required", nil)
	}
	records := make([]DirectoryRecord, 0)
	if rawRecordsValue, ok := output["records"]; ok && rawRecordsValue != nil {
		rawRecords, ok := rawRecordsValue.([]any)
		if !ok {
			return DirectoryResolution{}, invalidDirectory("Directory records must be a list", nil)
		}
		for _, raw := range rawRecords {
			record, ok := raw.(map[string]any)
			if !ok {
				return DirectoryResolution{}, invalidDirectory("Directory record must be an object", nil)
			}
			records = append(records, ProjectDirectoryRecord(record))
		}
	}
	nextHop, err := optionalDirectoryMap(output, "next_hop")
	if err != nil {
		return DirectoryResolution{}, err
	}
	selectedRoute, err := optionalDirectoryMap(output, "selected_route")
	if err != nil {
		return DirectoryResolution{}, err
	}
	routeCandidates, err := optionalDirectoryMapSlice(output, "route_candidates")
	if err != nil {
		return DirectoryResolution{}, err
	}
	authority, err := optionalDirectoryMap(output, "authority")
	if err != nil {
		return DirectoryResolution{}, err
	}
	cachePolicy, err := optionalDirectoryMap(output, "cache_policy")
	if err != nil {
		return DirectoryResolution{}, err
	}
	return DirectoryResolution{
		AnswerKind:      answerKind,
		CanonicalURA:    directoryString(output, "canonical_name"),
		OwnerURA:        directoryString(output, "owner_ura"),
		AbilityURA:      directoryString(output, "ability_ura"),
		RouteURA:        directoryString(output, "route_ura"),
		NextHop:         nextHop,
		SelectedRoute:   selectedRoute,
		RouteCandidates: routeCandidates,
		Records:         records,
		Negative:        negative,
		NextCursor:      directoryString(output, "next_cursor"),
		ReleaseProfile:  directoryString(output, "release_profile"),
		Authority:       authority,
		CachePolicy:     cachePolicy,
	}, nil
}

func optionalDirectoryMapSlice(value map[string]any, key string) ([]map[string]any, error) {
	raw, present := value[key]
	if !present || raw == nil {
		return nil, nil
	}
	rows, ok := raw.([]any)
	if !ok {
		return nil, invalidDirectory("Directory "+key+" must be a list", nil)
	}
	result := make([]map[string]any, 0, len(rows))
	for _, row := range rows {
		object, ok := row.(map[string]any)
		if !ok {
			return nil, invalidDirectory("Directory "+key+" item must be an object", nil)
		}
		result = append(result, directoryMap(object))
	}
	return result, nil
}

// ProjectDirectoryRecord projects one raw namespace.resolve Directory record
// while preserving its source facts for product read models.
func ProjectDirectoryRecord(raw map[string]any) DirectoryRecord {
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

func optionalDirectoryMap(value map[string]any, key string) (map[string]any, error) {
	raw, present := value[key]
	if !present || raw == nil {
		return nil, nil
	}
	object := directoryMap(raw)
	if object == nil {
		return nil, invalidDirectory("Directory "+key+" must be an object", nil)
	}
	return object, nil
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
