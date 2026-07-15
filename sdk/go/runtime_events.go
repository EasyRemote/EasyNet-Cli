package easynet

import (
	"context"
	"encoding/json"

	eventcore "easynet.run/cli/sdk/go/runtimeevents"
)

const (
	DefaultRuntimeEventPageLimit uint32 = 50
	MaxRuntimeEventPageLimit     uint32 = 500
)

type RuntimeEventStreamState = eventcore.StreamState

const (
	RuntimeEventStreamLive     = eventcore.StreamLive
	RuntimeEventStreamTerminal = eventcore.StreamTerminal
	RuntimeEventStreamFailed   = eventcore.StreamFailed
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

type RuntimeEventClient struct {
	provider RuntimeEventProvider
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

type RuntimeHandleEventProvider struct {
	runtime *RuntimeClient
}

func NewRuntimeHandleEventProvider(runtime *RuntimeClient) (*RuntimeHandleEventProvider, error) {
	if runtime == nil {
		return nil, invalidRuntimeClient("runtime client is required")
	}
	return &RuntimeHandleEventProvider{runtime: runtime}, nil
}

func (p *RuntimeHandleEventProvider) ReadEvents(ctx context.Context, request RuntimeEventReadRequest) (RuntimeEventPage, error) {
	if p == nil || p.runtime == nil {
		return RuntimeEventPage{}, invalidRuntimeClient("runtime event provider is not initialized")
	}
	if _, err := request.Handle.controlCapability(); err != nil {
		return RuntimeEventPage{}, err
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
		events = append(events, RuntimeEvent{
			Sequence: event.Sequence(),
			Kind:     event.Kind(),
			State:    event.State(),
			Terminal: event.Terminal(),
			Reason:   event.Reason(),
			Result:   event.Result(),
		})
		cursor.Sequence = event.Sequence()
	}
	state := RuntimeEventStreamLive
	if snapshot.Terminal() {
		state = RuntimeEventStreamTerminal
	}
	if err := eventcore.ValidatePageState(state, snapshot.Terminal()); err != nil {
		return RuntimeEventPage{}, invalidRuntimePayload(err.Error(), err)
	}
	return RuntimeEventPage{
		Events:   events,
		Cursor:   cursor,
		State:    state,
		Terminal: snapshot.Terminal(),
		Limit:    limit,
	}, nil
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
