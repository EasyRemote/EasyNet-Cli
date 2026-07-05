package easynet

import (
	"context"
	"encoding/json"
	"fmt"
	"strconv"
	"strings"
)

const (
	eventsAbilitySubscribeDirectory   = "federation.subscribe_directory_v2"
	eventsAbilitySubscribeDevices     = "events.device.subscribe"
	eventsAbilitySubscribeSessions    = "session.attach"
	eventsAbilitySubscribeInvocations = "events.invocation.subscribe"
)

var eventsCarrierArgKeys = map[string]struct{}{
	"caller_ura":         {},
	"callee_ura":         {},
	"subject_ura":        {},
	"descriptor_version": {},
	"nonce_base64":       {},
	"causal_context":     {},
	"metadata":           {},
}

// EventsRuntimeTransport lowers Events profile requests into Runtime Core
// Invocation drafts. Stream ownership stays with RuntimeClient.InvokeStream.
type EventsRuntimeTransport struct {
	runtime  *RuntimeClient
	identity *IdentityClient
}

func NewEventsRuntimeTransport(runtime *RuntimeClient, identity *IdentityClient) (*EventsRuntimeTransport, error) {
	if runtime == nil {
		return nil, invalidProfileClient(eventsProfile, "runtime client is required")
	}
	if identity == nil {
		return nil, invalidProfileClient(eventsProfile, "identity client is required")
	}
	return &EventsRuntimeTransport{runtime: runtime, identity: identity}, nil
}

func NewRuntimeEventClient(runtime *RuntimeClient, identity *IdentityClient) (*EventClient, error) {
	transport, err := NewEventsRuntimeTransport(runtime, identity)
	if err != nil {
		return nil, err
	}
	return NewEventClient(transport)
}

func (t *EventsRuntimeTransport) BuildDirectorySubscriptionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.buildSubscriptionInvocationJSON(ctx, requestJSON, EventStreamDirectory)
}

func (t *EventsRuntimeTransport) BuildDeviceSubscriptionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.buildSubscriptionInvocationJSON(ctx, requestJSON, EventStreamDevice)
}

func (t *EventsRuntimeTransport) BuildSessionSubscriptionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.buildSubscriptionInvocationJSON(ctx, requestJSON, EventStreamSession)
}

func (t *EventsRuntimeTransport) BuildInvocationSubscriptionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.buildSubscriptionInvocationJSON(ctx, requestJSON, EventStreamInvocation)
}

func (t *EventsRuntimeTransport) SubscribeDirectory(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(eventsProfile, "events directory subscriptions are opened through RuntimeClient.InvokeStream")
}

func (t *EventsRuntimeTransport) SubscribeDevices(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(eventsProfile, "events device subscriptions are opened through RuntimeClient.InvokeStream")
}

func (t *EventsRuntimeTransport) SubscribeSessions(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(eventsProfile, "events session subscriptions are opened through RuntimeClient.InvokeStream")
}

func (t *EventsRuntimeTransport) SubscribeInvocations(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(eventsProfile, "events invocation subscriptions are opened through RuntimeClient.InvokeStream")
}

func (t *EventsRuntimeTransport) ListDeviceEvents(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(eventsProfile, "events device history is not implemented by the runtime transport")
}

func (t *EventsRuntimeTransport) ProjectDirectoryEvent(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(eventsProfile, "events directory projection is daemon-owned and not implemented by the runtime transport")
}

func (t *EventsRuntimeTransport) ProjectDropReport(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(eventsProfile, "events drop-report projection is daemon-owned and not implemented by the runtime transport")
}

func (t *EventsRuntimeTransport) ProjectTerminal(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(eventsProfile, "events terminal projection is daemon-owned and not implemented by the runtime transport")
}

func (t *EventsRuntimeTransport) Close(ctx context.Context) error {
	return nil
}

func (t *EventsRuntimeTransport) buildSubscriptionInvocationJSON(ctx context.Context, requestJSON []byte, stream EventStreamKind) ([]byte, error) {
	draft, err := t.buildSubscriptionInvocation(ctx, requestJSON, stream)
	if err != nil {
		return nil, err
	}
	raw, err := json.Marshal(draft)
	if err != nil {
		return nil, invalidProfilePayload(eventsProfile, fmt.Sprintf("encode events invocation: %v", err), err)
	}
	return raw, nil
}

func (t *EventsRuntimeTransport) buildSubscriptionInvocation(ctx context.Context, requestJSON []byte, stream EventStreamKind) (InvocationDraft, error) {
	if t == nil || t.runtime == nil || t.identity == nil {
		return InvocationDraft{}, invalidProfileClient(eventsProfile, "events runtime transport is not initialized")
	}
	if ctx == nil {
		return InvocationDraft{}, invalidProfileClient(eventsProfile, "context is required")
	}
	request, payload, err := decodeEventsSubscriptionForRuntime(requestJSON, stream)
	if err != nil {
		return InvocationDraft{}, err
	}
	abilityName, err := eventsSubscriptionAbility(stream)
	if err != nil {
		return InvocationDraft{}, err
	}
	descriptorRef, err := t.identity.OwnerAbilityDescriptorRef(ctx, request.CalleeURA, abilityName, request.DescriptorVersion)
	if err != nil {
		return InvocationDraft{}, err
	}
	return NewInvocationBuilder().
		WithCallerURA(request.CallerURA).
		WithCalleeURA(request.CalleeURA).
		WithDescriptorRef(descriptorRef).
		WithSubjectURA(request.SubjectURA).
		WithNonceBase64(request.NonceBase64).
		WithCausalContext(request.CausalContext).
		WithJSONArgs(eventsRuntimeArgs(payload, stream, abilityName)).
		WithContentType("application/json").
		WithMetadata(eventsRuntimeMetadata(request.Metadata, abilityName)).
		Build()
}

func decodeEventsSubscriptionForRuntime(requestJSON []byte, expected EventStreamKind) (EventsSubscriptionRequest, map[string]any, error) {
	var request EventsSubscriptionRequest
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		return EventsSubscriptionRequest{}, nil, invalidProfilePayload(eventsProfile, fmt.Sprintf("decode events subscription request: %v", err), err)
	}
	normalized, err := normalizeEventsSubscriptionRequest(request, expected)
	if err != nil {
		return EventsSubscriptionRequest{}, nil, err
	}
	var payload map[string]any
	if err := json.Unmarshal(requestJSON, &payload); err != nil {
		return EventsSubscriptionRequest{}, nil, invalidProfilePayload(eventsProfile, fmt.Sprintf("decode events subscription payload: %v", err), err)
	}
	if payload == nil {
		return EventsSubscriptionRequest{}, nil, invalidProfilePayload(eventsProfile, "events subscription request must be an object", nil)
	}
	return normalized, payload, nil
}

func eventsSubscriptionAbility(stream EventStreamKind) (string, error) {
	switch stream {
	case EventStreamDirectory:
		return eventsAbilitySubscribeDirectory, nil
	case EventStreamDevice:
		return eventsAbilitySubscribeDevices, nil
	case EventStreamSession:
		return eventsAbilitySubscribeSessions, nil
	case EventStreamInvocation:
		return eventsAbilitySubscribeInvocations, nil
	default:
		return "", invalidProfilePayload(eventsProfile, "unsupported event stream", nil)
	}
}

func eventsRuntimeArgs(payload map[string]any, stream EventStreamKind, abilityName string) map[string]any {
	args := map[string]any{}
	if stream != EventStreamSession {
		args["stream"] = string(stream)
		args["daemon_ability"] = abilityName
	}
	for key, value := range payload {
		if _, carrier := eventsCarrierArgKeys[key]; carrier {
			continue
		}
		if isEmptyEventRuntimeArg(value) {
			continue
		}
		if key == "resume_cursor" {
			if token := eventRuntimeResumeToken(value); token != "" {
				if stream == EventStreamSession {
					if _, sequence, ok := parseEventRuntimeResumeToken(token); ok {
						args["since_seq"] = sequence
					}
				} else {
					args[key] = token
				}
			}
			continue
		}
		if stream == EventStreamSession && key == "stream" {
			continue
		}
		args[key] = value
	}
	return args
}

func eventRuntimeResumeToken(value any) string {
	switch typed := value.(type) {
	case string:
		return strings.TrimSpace(typed)
	case map[string]any:
		token, _ := typed["token"].(string)
		if strings.TrimSpace(token) != "" {
			return strings.TrimSpace(token)
		}
		stream, _ := typed["stream"].(string)
		sequence, ok := numericUint64(typed["sequence"])
		if strings.TrimSpace(stream) == "" || !ok {
			return ""
		}
		return fmt.Sprintf("%s:%d", strings.TrimSpace(stream), sequence)
	default:
		return ""
	}
}

func parseEventRuntimeResumeToken(token string) (string, uint64, bool) {
	parts := strings.Split(strings.TrimSpace(token), ":")
	if len(parts) != 2 || strings.TrimSpace(parts[0]) == "" {
		return "", 0, false
	}
	sequence, err := strconv.ParseUint(strings.TrimSpace(parts[1]), 10, 64)
	if err != nil {
		return "", 0, false
	}
	return strings.TrimSpace(parts[0]), sequence, true
}

func numericUint64(value any) (uint64, bool) {
	switch typed := value.(type) {
	case float64:
		if typed < 0 || typed != float64(uint64(typed)) {
			return 0, false
		}
		return uint64(typed), true
	case int:
		if typed < 0 {
			return 0, false
		}
		return uint64(typed), true
	case uint64:
		return typed, true
	default:
		return 0, false
	}
}

func isEmptyEventRuntimeArg(value any) bool {
	switch typed := value.(type) {
	case nil:
		return true
	case string:
		return strings.TrimSpace(typed) == ""
	case float64:
		return typed == 0
	case int:
		return typed == 0
	case map[string]any:
		return len(typed) == 0
	default:
		return false
	}
}

func eventsRuntimeMetadata(base map[string]any, abilityName string) map[string]any {
	metadata := map[string]any{}
	for key, value := range base {
		metadata[key] = value
	}
	metadata["profile"] = eventsProfile
	metadata["system_ability"] = abilityName
	metadata["carrier_owner"] = "daemon_sdk"
	return metadata
}

func sdkProfileNotImplemented(profile string, message string) error {
	return &SDKError{
		Code:      ErrNotImplemented,
		Stage:     profile,
		Retry:     RetryNever,
		Retryable: false,
		Message:   message,
		Details: map[string]any{
			"profile": profile,
		},
	}
}
