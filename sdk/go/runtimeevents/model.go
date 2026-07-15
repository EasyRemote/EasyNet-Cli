// Package runtimeevents defines the provider-neutral runtime event lifecycle.
package runtimeevents

import (
	"fmt"
	"strings"
)

type StreamState string

const (
	StreamLive     StreamState = "Live"
	StreamTerminal StreamState = "Terminal"
	StreamFailed   StreamState = "Failed"
)

// TransitionStream enforces the shared event-feed lifecycle. Terminal and
// failed states are absorbing; a live feed may only remain live or terminate.
func TransitionStream(current, next StreamState) error {
	if current == "" {
		current = StreamLive
	}
	switch current {
	case StreamLive:
		if next == StreamLive || next == StreamTerminal || next == StreamFailed {
			return nil
		}
	case StreamTerminal, StreamFailed:
		if next == current {
			return nil
		}
	}
	return fmt.Errorf("runtime event stream cannot transition from %q to %q", current, next)
}

func ValidatePageState(state StreamState, terminal bool) error {
	if state == "" {
		return fmt.Errorf("runtime event stream state is required")
	}
	wantTerminal := state == StreamTerminal || state == StreamFailed
	if terminal != wantTerminal {
		return fmt.Errorf("runtime event terminal flag does not match state %q", state)
	}
	return TransitionStream(StreamLive, state)
}

type Topic string

type CursorProjection string

const (
	CursorProjectionNone     CursorProjection = ""
	CursorProjectionToken    CursorProjection = "token"
	CursorProjectionSequence CursorProjection = "sequence"
)

type Route struct {
	Topic            Topic
	Ability          string
	StreamArgument   string
	AbilityArgument  string
	CursorArgument   string
	CursorProjection CursorProjection
}

type ResumeCursor struct {
	Topic    Topic
	Sequence uint64
	Token    string
}

type Subscription struct {
	Topic               Topic
	Parameters          map[string]any
	HeartbeatIntervalMS int
	Metadata            map[string]any
	ResumeCursor        *ResumeCursor
}

type SubscriptionProjection struct {
	Ability   string
	Arguments map[string]any
	Metadata  map[string]any
}

type RouteCatalog struct {
	routes map[Topic]Route
}

func NewRouteCatalog(routes []Route) (*RouteCatalog, error) {
	if len(routes) == 0 {
		return nil, fmt.Errorf("runtime event routes are required")
	}
	catalog := &RouteCatalog{routes: make(map[Topic]Route, len(routes))}
	for _, route := range routes {
		topic := Topic(strings.TrimSpace(string(route.Topic)))
		if topic == "" {
			return nil, fmt.Errorf("runtime event route topic is required")
		}
		ability := strings.TrimSpace(route.Ability)
		if ability == "" {
			return nil, fmt.Errorf("runtime event route ability is required for topic %q", topic)
		}
		if _, exists := catalog.routes[topic]; exists {
			return nil, fmt.Errorf("duplicate runtime event route for topic %q", topic)
		}
		route.Topic = topic
		route.Ability = ability
		catalog.routes[topic] = route
	}
	return catalog, nil
}

func (c *RouteCatalog) Resolve(topic Topic) (Route, error) {
	if c == nil {
		return Route{}, fmt.Errorf("runtime event route catalog is not initialized")
	}
	normalized := Topic(strings.TrimSpace(string(topic)))
	if normalized == "" {
		return Route{}, fmt.Errorf("runtime event topic is required")
	}
	route, ok := c.routes[normalized]
	if !ok {
		return Route{}, fmt.Errorf("unsupported runtime event topic %q", normalized)
	}
	return route, nil
}

func (c *RouteCatalog) Build(subscription Subscription) (SubscriptionProjection, error) {
	route, err := c.Resolve(subscription.Topic)
	if err != nil {
		return SubscriptionProjection{}, err
	}
	args := cloneMap(subscription.Parameters, 4)
	if route.StreamArgument != "" {
		args[route.StreamArgument] = string(route.Topic)
	}
	if route.AbilityArgument != "" {
		args[route.AbilityArgument] = route.Ability
	}
	if subscription.HeartbeatIntervalMS > 0 {
		args["heartbeat_interval_ms"] = subscription.HeartbeatIntervalMS
	}
	if subscription.ResumeCursor != nil && route.CursorArgument != "" {
		if subscription.ResumeCursor.Topic != "" && subscription.ResumeCursor.Topic != route.Topic {
			return SubscriptionProjection{}, fmt.Errorf(
				"runtime event cursor topic %q does not match subscription topic %q",
				subscription.ResumeCursor.Topic,
				route.Topic,
			)
		}
		switch route.CursorProjection {
		case CursorProjectionToken:
			token := strings.TrimSpace(subscription.ResumeCursor.Token)
			if token == "" {
				token = fmt.Sprintf("%s:%d", route.Topic, subscription.ResumeCursor.Sequence)
			}
			args[route.CursorArgument] = token
		case CursorProjectionSequence:
			args[route.CursorArgument] = subscription.ResumeCursor.Sequence
		case CursorProjectionNone:
		default:
			return SubscriptionProjection{}, fmt.Errorf("unsupported runtime event cursor projection %q", route.CursorProjection)
		}
	}
	metadata := cloneMap(subscription.Metadata, 1)
	metadata["system_ability"] = route.Ability
	return SubscriptionProjection{
		Ability:   route.Ability,
		Arguments: args,
		Metadata:  metadata,
	}, nil
}

func cloneMap(input map[string]any, extra int) map[string]any {
	output := make(map[string]any, len(input)+extra)
	for key, value := range input {
		output[key] = value
	}
	return output
}
