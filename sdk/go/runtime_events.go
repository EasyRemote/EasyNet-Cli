package easynet

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
)

const (
	DefaultRuntimeEventPageLimit uint32 = 50
	MaxRuntimeEventPageLimit     uint32 = 500
)

type RuntimeEventStreamState string

const (
	RuntimeEventStreamLive     RuntimeEventStreamState = "Live"
	RuntimeEventStreamTerminal RuntimeEventStreamState = "Terminal"
	RuntimeEventStreamFailed   RuntimeEventStreamState = "Failed"
)

type RuntimeEventCursor struct {
	Sequence uint64 `json:"sequence"`
}

func NewRuntimeEventCursor(sequence uint64) RuntimeEventCursor {
	return RuntimeEventCursor{Sequence: sequence}
}

type RuntimeEvent struct {
	Sequence uint64          `json:"sequence"`
	Kind     string          `json:"kind"`
	State    string          `json:"state"`
	Terminal bool            `json:"terminal"`
	Reason   string          `json:"reason,omitempty"`
	Result   json.RawMessage `json:"result,omitempty"`
}

type RuntimeEventReadRequest struct {
	Handle InvocationHandle    `json:"-"`
	Cursor *RuntimeEventCursor `json:"cursor,omitempty"`
	Limit  uint32              `json:"limit,omitempty"`
}

type RuntimeEventStreamKind string

const (
	RuntimeEventStreamDirectory  RuntimeEventStreamKind = "directory"
	RuntimeEventStreamDevice     RuntimeEventStreamKind = "device"
	RuntimeEventStreamSession    RuntimeEventStreamKind = "session"
	RuntimeEventStreamInvocation RuntimeEventStreamKind = "invocation"
)

type RuntimeEventSubscriptionCursor struct {
	Stream   string `json:"stream"`
	Sequence uint64 `json:"sequence"`
	Token    string `json:"token,omitempty"`
}

func (c RuntimeEventSubscriptionCursor) ResumeToken() string {
	if strings.TrimSpace(c.Token) != "" {
		return strings.TrimSpace(c.Token)
	}
	if strings.TrimSpace(c.Stream) == "" {
		return ""
	}
	return fmt.Sprintf("%s:%d", strings.TrimSpace(c.Stream), c.Sequence)
}

type RuntimeEventSubscriptionRequest struct {
	Call                RuntimeCallContext              `json:"call"`
	Stream              RuntimeEventStreamKind          `json:"stream,omitempty"`
	Realm               string                          `json:"realm,omitempty"`
	OwnerURA            string                          `json:"owner_ura,omitempty"`
	DeviceURA           string                          `json:"device_ura,omitempty"`
	AgentURA            string                          `json:"agent_ura,omitempty"`
	SessionID           string                          `json:"session_id,omitempty"`
	InvocationID        string                          `json:"invocation_id,omitempty"`
	ResumeCursor        *RuntimeEventSubscriptionCursor `json:"resume_cursor,omitempty"`
	HeartbeatIntervalMS int                             `json:"heartbeat_interval_ms,omitempty"`
}

type RuntimeEventPage struct {
	Events   []RuntimeEvent          `json:"events"`
	Cursor   RuntimeEventCursor      `json:"cursor"`
	State    RuntimeEventStreamState `json:"state"`
	Terminal bool                    `json:"terminal"`
	Limit    uint32                  `json:"limit"`
}

type RuntimeEventProvider interface {
	ReadEvents(context.Context, RuntimeEventReadRequest) (RuntimeEventPage, error)
}

type RuntimeEventSubscriptionProvider interface {
	BuildSubscription(context.Context, RuntimeEventSubscriptionRequest) (InvocationDraft, error)
}

type RuntimeEventClient struct {
	provider RuntimeEventProvider
}

type RuntimeEventSubscriptionClient struct {
	provider RuntimeEventSubscriptionProvider
}

func NewRuntimeEventClient(provider RuntimeEventProvider) (*RuntimeEventClient, error) {
	if provider == nil {
		return nil, invalidRuntimeClient("runtime event provider is required")
	}
	return &RuntimeEventClient{provider: provider}, nil
}

func (c *RuntimeEventClient) Read(ctx context.Context, request RuntimeEventReadRequest) (RuntimeEventPage, error) {
	if c == nil || c.provider == nil {
		return RuntimeEventPage{}, invalidRuntimeClient("runtime event client is not initialized")
	}
	if ctx == nil {
		return RuntimeEventPage{}, invalidRuntimeClient("context is required")
	}
	return c.provider.ReadEvents(ctx, request)
}

func NewRuntimeEventSubscriptionClient(provider RuntimeEventSubscriptionProvider) (*RuntimeEventSubscriptionClient, error) {
	if provider == nil {
		return nil, invalidRuntimeClient("runtime event subscription provider is required")
	}
	return &RuntimeEventSubscriptionClient{provider: provider}, nil
}

func (c *RuntimeEventSubscriptionClient) Build(ctx context.Context, request RuntimeEventSubscriptionRequest) (InvocationDraft, error) {
	if c == nil || c.provider == nil {
		return InvocationDraft{}, invalidRuntimeClient("runtime event subscription client is not initialized")
	}
	if ctx == nil {
		return InvocationDraft{}, invalidRuntimeClient("context is required")
	}
	return c.provider.BuildSubscription(ctx, request)
}

type RuntimeHandleEventProvider struct {
	runtime *RuntimeClient
}

type RuntimeAbilityEventSubscriptionProvider struct {
	ability *RuntimeAbilityClient
}

func NewRuntimeHandleEventProvider(runtime *RuntimeClient) (*RuntimeHandleEventProvider, error) {
	if runtime == nil {
		return nil, invalidRuntimeClient("runtime client is required")
	}
	return &RuntimeHandleEventProvider{runtime: runtime}, nil
}

func NewRuntimeAbilityEventSubscriptionProvider(ability *RuntimeAbilityClient) (*RuntimeAbilityEventSubscriptionProvider, error) {
	if ability == nil {
		return nil, invalidRuntimeClient("runtime ability client is required")
	}
	return &RuntimeAbilityEventSubscriptionProvider{ability: ability}, nil
}

func (p *RuntimeHandleEventProvider) ReadEvents(ctx context.Context, request RuntimeEventReadRequest) (RuntimeEventPage, error) {
	if p == nil || p.runtime == nil {
		return RuntimeEventPage{}, invalidRuntimeClient("runtime event provider is not initialized")
	}
	if request.Handle.HandleID() == 0 {
		return RuntimeEventPage{}, invalidRuntimePayload("handle_id is required", nil)
	}
	limit, err := normalizeRuntimeEventLimit(request.Limit)
	if err != nil {
		return RuntimeEventPage{}, err
	}
	after := uint64(0)
	if request.Cursor != nil {
		after = request.Cursor.Sequence
	}
	snapshot, err := p.runtime.Events(ctx, request.Handle)
	if err != nil {
		return RuntimeEventPage{}, err
	}
	events := make([]RuntimeEvent, 0, len(snapshot.Events()))
	cursor := RuntimeEventCursor{Sequence: after}
	for _, event := range snapshot.Events() {
		if event.Sequence() <= after {
			continue
		}
		if uint32(len(events)) >= limit {
			break
		}
		projected := RuntimeEvent{
			Sequence: event.Sequence(),
			Kind:     event.Kind(),
			State:    event.State(),
			Terminal: event.Terminal(),
			Reason:   event.Reason(),
			Result:   event.Result(),
		}
		events = append(events, projected)
		cursor.Sequence = event.Sequence()
	}
	state := RuntimeEventStreamLive
	if snapshot.Terminal() {
		state = RuntimeEventStreamTerminal
	}
	return RuntimeEventPage{
		Events:   events,
		Cursor:   cursor,
		State:    state,
		Terminal: snapshot.Terminal(),
		Limit:    limit,
	}, nil
}

func (p *RuntimeAbilityEventSubscriptionProvider) BuildSubscription(ctx context.Context, request RuntimeEventSubscriptionRequest) (InvocationDraft, error) {
	if p == nil || p.ability == nil {
		return InvocationDraft{}, invalidRuntimeClient("runtime event subscription provider is not initialized")
	}
	ability, err := RuntimeEventSubscriptionAbility(request.Stream)
	if err != nil {
		return InvocationDraft{}, err
	}
	args := map[string]any{}
	if request.Stream != RuntimeEventStreamSession {
		args["stream"] = string(request.Stream)
		args["daemon_ability"] = ability
	}
	putRuntimeEventString(args, "realm", request.Realm)
	putRuntimeEventString(args, "owner_ura", request.OwnerURA)
	putRuntimeEventString(args, "device_ura", request.DeviceURA)
	putRuntimeEventString(args, "agent_ura", request.AgentURA)
	putRuntimeEventString(args, "session_id", request.SessionID)
	putRuntimeEventString(args, "invocation_id", request.InvocationID)
	if request.HeartbeatIntervalMS > 0 {
		args["heartbeat_interval_ms"] = request.HeartbeatIntervalMS
	}
	if request.ResumeCursor != nil {
		if err := validateRuntimeEventResumeCursor(request.Stream, *request.ResumeCursor); err != nil {
			return InvocationDraft{}, err
		}
		if request.Stream == RuntimeEventStreamSession {
			args["since_seq"] = request.ResumeCursor.Sequence
		} else if token := request.ResumeCursor.ResumeToken(); token != "" {
			args["resume_cursor"] = token
		}
	}
	call := request.Call
	metadata := cloneRuntimeEventMetadata(call.Metadata)
	metadata["sdk_profile"] = "runtime_events"
	metadata["system_ability"] = ability
	call.Metadata = metadata
	return p.ability.Build(ctx, call, ability, args)
}

func RuntimeEventSubscriptionAbility(stream RuntimeEventStreamKind) (string, error) {
	switch stream {
	case RuntimeEventStreamDirectory:
		return "federation.subscribe_directory_v2", nil
	case RuntimeEventStreamDevice:
		return "events.device.subscribe", nil
	case RuntimeEventStreamSession:
		return "session.attach", nil
	case RuntimeEventStreamInvocation:
		return "events.invocation.subscribe", nil
	default:
		return "", invalidRuntimePayload(fmt.Sprintf("unsupported runtime event stream %q", stream), nil)
	}
}

func normalizeRuntimeEventLimit(limit uint32) (uint32, error) {
	if limit == 0 {
		return DefaultRuntimeEventPageLimit, nil
	}
	if limit > MaxRuntimeEventPageLimit {
		return 0, invalidRuntimePayload("runtime event page limit exceeds maximum", nil)
	}
	return limit, nil
}

func validateRuntimeEventResumeCursor(stream RuntimeEventStreamKind, cursor RuntimeEventSubscriptionCursor) error {
	cursorStream := strings.TrimSpace(cursor.Stream)
	if cursorStream == "" {
		return invalidRuntimePayload("runtime event resume cursor stream is required", nil)
	}
	if cursorStream != string(stream) {
		return invalidRuntimePayload("runtime event resume cursor stream does not match subscription stream", nil)
	}
	if cursor.Token != "" && strings.TrimSpace(cursor.Token) != cursor.Token {
		return invalidRuntimePayload("runtime event resume cursor token must be canonical", nil)
	}
	return nil
}

func cloneRuntimeEventMetadata(input map[string]any) map[string]any {
	output := make(map[string]any, len(input)+2)
	for key, value := range input {
		output[key] = value
	}
	return output
}

func putRuntimeEventString(values map[string]any, key string, value string) {
	if value = strings.TrimSpace(value); value != "" {
		values[key] = value
	}
}
