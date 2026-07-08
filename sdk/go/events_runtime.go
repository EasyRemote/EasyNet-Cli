package easynet

import (
	"context"
	"encoding/json"
	"fmt"
	"strconv"
	"strings"
	"sync"
	"time"
)

const (
	eventsAbilitySubscribeDirectory   = "federation.subscribe_directory_v2"
	eventsAbilitySubscribeDevices     = "events.device.subscribe"
	eventsAbilitySubscribeSessions    = "session.attach"
	eventsAbilitySubscribeInvocations = "events.invocation.subscribe"
	eventsAbilityDeviceHistory        = "events.device.history"
)

var eventsCarrierArgKeys = map[string]struct{}{
	"caller_ura":         {},
	"callee_ura":         {},
	"subject_ura":        {},
	"descriptor_version": {},
	"nonce_base64":       {},
	"causal_context":     {},
	"metadata":           {},
	"filter":             {},
}

// EventsRuntimeTransport lowers Events profile requests into Runtime Core
// Invocation drafts. Stream ownership stays with RuntimeClient.InvokeStream.
type EventsRuntimeTransport struct {
	runtime            *RuntimeClient
	identity           *IdentityClient
	projectionProvider EventsProjectionProvider
	mu                 sync.Mutex
	streams            map[string]*StreamHandle
}

// EventsProjectionProvider supplies daemon-owned EventFrame projections to the
// Runtime-backed Events facade. Runtime Core remains responsible for opening
// and draining streams.
type EventsProjectionProvider interface {
	ProjectDirectoryEvent(ctx context.Context, eventJSON []byte) ([]byte, error)
	ProjectLiveEvent(ctx context.Context, eventJSON []byte) ([]byte, error)
	ProjectDropReport(ctx context.Context, dropJSON []byte) ([]byte, error)
	ProjectTerminal(ctx context.Context, terminalJSON []byte) ([]byte, error)
}

func NewEventsRuntimeTransport(runtime *RuntimeClient, identity *IdentityClient) (*EventsRuntimeTransport, error) {
	return NewEventsRuntimeTransportWithProjectionProvider(runtime, identity, nil)
}

func NewEventsRuntimeTransportWithProjectionProvider(runtime *RuntimeClient, identity *IdentityClient, projectionProvider EventsProjectionProvider) (*EventsRuntimeTransport, error) {
	if runtime == nil {
		return nil, invalidProfileClient(eventsProfile, "runtime client is required")
	}
	if identity == nil {
		return nil, invalidProfileClient(eventsProfile, "identity client is required")
	}
	return &EventsRuntimeTransport{
		runtime:            runtime,
		identity:           identity,
		projectionProvider: projectionProvider,
		streams:            map[string]*StreamHandle{},
	}, nil
}

func NewRuntimeEventClient(runtime *RuntimeClient, identity *IdentityClient) (*EventClient, error) {
	return NewRuntimeEventClientWithProjectionProvider(runtime, identity, nil)
}

func NewRuntimeEventClientWithProjectionProvider(runtime *RuntimeClient, identity *IdentityClient, projectionProvider EventsProjectionProvider) (*EventClient, error) {
	transport, err := NewEventsRuntimeTransportWithProjectionProvider(runtime, identity, projectionProvider)
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

func (t *EventsRuntimeTransport) SubscribeDirectory(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.openSubscriptionStream(ctx, requestJSON, EventStreamDirectory)
}

func (t *EventsRuntimeTransport) SubscribeDevices(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.openSubscriptionStream(ctx, requestJSON, EventStreamDevice)
}

func (t *EventsRuntimeTransport) SubscribeSessions(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.openSubscriptionStream(ctx, requestJSON, EventStreamSession)
}

func (t *EventsRuntimeTransport) SubscribeInvocations(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.openSubscriptionStream(ctx, requestJSON, EventStreamInvocation)
}

func (t *EventsRuntimeTransport) ListDeviceEvents(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeEventsDeviceHistoryForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	draft, err := t.buildDeviceHistoryInvocation(ctx, request)
	if err != nil {
		return nil, err
	}
	result, err := t.runtime.Invoke(ctx, draft)
	if err != nil {
		return nil, err
	}
	if !result.OK() {
		return nil, eventsInvocationFailureError(result)
	}
	outputJSON := result.OutputJSON()
	if len(outputJSON) == 0 || string(outputJSON) == "null" {
		return nil, invalidProfilePayload(eventsProfile, "events device history output_json is required", nil)
	}
	return projectRuntimeDeviceEventPage(request, outputJSON)
}

func (t *EventsRuntimeTransport) ProjectDirectoryEvent(ctx context.Context, eventJSON []byte) ([]byte, error) {
	provider, err := t.requireProjectionProvider(ctx)
	if err != nil {
		return nil, err
	}
	raw, err := provider.ProjectDirectoryEvent(ctx, eventJSON)
	if err != nil {
		return nil, err
	}
	if _, err := NewEventFrameFromJSON(raw); err != nil {
		return nil, err
	}
	return raw, nil
}

func (t *EventsRuntimeTransport) ProjectLiveEvent(ctx context.Context, eventJSON []byte) ([]byte, error) {
	provider, err := t.requireProjectionProvider(ctx)
	if err != nil {
		return nil, err
	}
	raw, err := provider.ProjectLiveEvent(ctx, eventJSON)
	if err != nil {
		return nil, err
	}
	if _, err := NewEventFrameFromJSON(raw); err != nil {
		return nil, err
	}
	return raw, nil
}

func (t *EventsRuntimeTransport) ProjectDropReport(ctx context.Context, dropJSON []byte) ([]byte, error) {
	provider, err := t.requireProjectionProvider(ctx)
	if err != nil {
		return nil, err
	}
	raw, err := provider.ProjectDropReport(ctx, dropJSON)
	if err != nil {
		return nil, err
	}
	if _, err := NewEventFrameFromJSON(raw); err != nil {
		return nil, err
	}
	return raw, nil
}

func (t *EventsRuntimeTransport) ProjectTerminal(ctx context.Context, terminalJSON []byte) ([]byte, error) {
	provider, err := t.requireProjectionProvider(ctx)
	if err != nil {
		return nil, err
	}
	raw, err := provider.ProjectTerminal(ctx, terminalJSON)
	if err != nil {
		return nil, err
	}
	if _, err := NewEventFrameFromJSON(raw); err != nil {
		return nil, err
	}
	return raw, nil
}

func (t *EventsRuntimeTransport) Close(ctx context.Context) error {
	if ctx == nil {
		return invalidProfileClient(eventsProfile, "context is required")
	}
	if t == nil {
		return nil
	}
	t.mu.Lock()
	streams := make([]*StreamHandle, 0, len(t.streams))
	for id, stream := range t.streams {
		streams = append(streams, stream)
		delete(t.streams, id)
	}
	t.mu.Unlock()
	var first error
	for _, stream := range streams {
		if err := stream.Close(ctx); err != nil && first == nil {
			first = err
		}
	}
	return first
}

func (t *EventsRuntimeTransport) requireProjectionProvider(ctx context.Context) (EventsProjectionProvider, error) {
	if ctx == nil {
		return nil, invalidProfileClient(eventsProfile, "context is required")
	}
	if t == nil || t.projectionProvider == nil {
		return nil, invalidProfileClient(eventsProfile, "events runtime projection provider is required")
	}
	return t.projectionProvider, nil
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
	subjectURA, err := descriptorBoundSubjectURA(ctx, t.identity, request.SubjectURA, abilityName)
	if err != nil {
		return InvocationDraft{}, err
	}
	return NewInvocationBuilder().
		WithCallerURA(request.CallerURA).
		WithCalleeURA(request.CalleeURA).
		WithDescriptorRef(descriptorRef).
		WithSubjectURA(subjectURA).
		WithNonceBase64(request.NonceBase64).
		WithCausalContext(request.CausalContext).
		WithJSONArgs(eventsRuntimeArgs(payload, stream, abilityName)).
		WithContentType("application/json").
		WithMetadata(eventsRuntimeMetadata(request.Metadata, abilityName)).
		Build()
}

func (t *EventsRuntimeTransport) openSubscriptionStream(ctx context.Context, requestJSON []byte, stream EventStreamKind) ([]byte, error) {
	request, _, err := decodeEventsSubscriptionForRuntime(requestJSON, stream)
	if err != nil {
		return nil, err
	}
	draft, err := t.buildSubscriptionInvocation(ctx, requestJSON, stream)
	if err != nil {
		return nil, err
	}
	handle, err := t.runtime.InvokeStream(ctx, draft)
	if err != nil {
		return nil, err
	}
	t.mu.Lock()
	if t.streams == nil {
		t.streams = map[string]*StreamHandle{}
	}
	t.streams[handle.StreamID()] = handle
	t.mu.Unlock()
	return eventsRuntimeStreamOpenJSON(stream, request, handle)
}

func (t *EventsRuntimeTransport) bindEventStreamHandle(stream EventStream) EventStream {
	if t == nil || stream.StreamID == "" {
		return stream
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	stream.handle = t.streams[stream.StreamID]
	if liveEventProjectionSupported(EventStreamKind(stream.Stream)) && t.projectionProvider != nil {
		stream.projectLive = func(ctx context.Context, input EventProjectionInput) (EventFrame, error) {
			requestJSON, err := json.Marshal(input)
			if err != nil {
				return EventFrame{}, invalidProfilePayload(eventsProfile, fmt.Sprintf("encode events projection input: %v", err), err)
			}
			raw, err := t.ProjectLiveEvent(ctx, requestJSON)
			if err != nil {
				return EventFrame{}, err
			}
			return NewEventFrameFromJSON(raw)
		}
	}
	stream.release = t.releaseEventStreamHandle
	return stream
}

func liveEventProjectionSupported(stream EventStreamKind) bool {
	return stream == EventStreamDirectory || stream == EventStreamDevice || stream == EventStreamInvocation
}

func (t *EventsRuntimeTransport) releaseEventStreamHandle(streamID string) {
	if t == nil || streamID == "" {
		return
	}
	t.mu.Lock()
	delete(t.streams, streamID)
	t.mu.Unlock()
}

func (t *EventsRuntimeTransport) buildDeviceHistoryInvocation(ctx context.Context, request EventsDeviceEventListRequest) (InvocationDraft, error) {
	if t == nil || t.runtime == nil || t.identity == nil {
		return InvocationDraft{}, invalidProfileClient(eventsProfile, "events runtime transport is not initialized")
	}
	if ctx == nil {
		return InvocationDraft{}, invalidProfileClient(eventsProfile, "context is required")
	}
	descriptorRef, err := t.identity.OwnerAbilityDescriptorRef(ctx, request.CalleeURA, eventsAbilityDeviceHistory, request.DescriptorVersion)
	if err != nil {
		return InvocationDraft{}, err
	}
	subjectURA, err := descriptorBoundSubjectURA(ctx, t.identity, request.SubjectURA, eventsAbilityDeviceHistory)
	if err != nil {
		return InvocationDraft{}, err
	}
	return NewInvocationBuilder().
		WithCallerURA(request.CallerURA).
		WithCalleeURA(request.CalleeURA).
		WithDescriptorRef(descriptorRef).
		WithSubjectURA(subjectURA).
		WithNonceBase64(request.NonceBase64).
		WithCausalContext(request.CausalContext).
		WithJSONArgs(eventsDeviceHistoryRuntimeArgs(request)).
		WithContentType("application/json").
		WithMetadata(eventsRuntimeMetadata(request.Metadata, eventsAbilityDeviceHistory)).
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
	normalizedJSON, err := json.Marshal(normalized)
	if err != nil {
		return EventsSubscriptionRequest{}, nil, invalidProfilePayload(eventsProfile, fmt.Sprintf("encode normalized events subscription: %v", err), err)
	}
	var payload map[string]any
	if err := json.Unmarshal(normalizedJSON, &payload); err != nil {
		return EventsSubscriptionRequest{}, nil, invalidProfilePayload(eventsProfile, fmt.Sprintf("decode events subscription payload: %v", err), err)
	}
	if payload == nil {
		return EventsSubscriptionRequest{}, nil, invalidProfilePayload(eventsProfile, "events subscription request must be an object", nil)
	}
	return normalized, payload, nil
}

func decodeEventsDeviceHistoryForRuntime(requestJSON []byte) (EventsDeviceEventListRequest, error) {
	var request EventsDeviceEventListRequest
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		return EventsDeviceEventListRequest{}, invalidProfilePayload(eventsProfile, fmt.Sprintf("decode events device history request: %v", err), err)
	}
	normalized, err := normalizeEventsDeviceEventListRequest(request)
	if err != nil {
		return EventsDeviceEventListRequest{}, err
	}
	return normalized, nil
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

func eventsDeviceHistoryRuntimeArgs(request EventsDeviceEventListRequest) map[string]any {
	args := map[string]any{
		"stream":         string(EventStreamDevice),
		"daemon_ability": eventsAbilityDeviceHistory,
		"limit":          request.Limit,
	}
	if strings.TrimSpace(request.DeviceURA) != "" {
		args["device_ura"] = strings.TrimSpace(request.DeviceURA)
	}
	if strings.TrimSpace(request.Cursor) != "" {
		args["cursor"] = strings.TrimSpace(request.Cursor)
	}
	return args
}

func eventsRuntimeStreamOpenJSON(stream EventStreamKind, request EventsSubscriptionRequest, handle *StreamHandle) ([]byte, error) {
	if handle == nil {
		return nil, invalidProfileClient(eventsProfile, "runtime stream handle is required")
	}
	abilityName, err := eventsSubscriptionAbility(stream)
	if err != nil {
		return nil, err
	}
	metadata := map[string]any{
		"profile":             eventsProfile,
		"source":              "runtime_stream",
		"system_ability":      abilityName,
		"runtime_stream_id":   handle.StreamID(),
		"max_buffered_events": handle.MaxBufferedEvents(),
	}
	for key, value := range request.Metadata {
		if value != nil {
			metadata[key] = value
		}
	}
	resumeToken := ""
	if request.ResumeCursor != nil {
		resumeToken = request.ResumeCursor.ResumeToken()
	}
	return json.Marshal(map[string]any{
		"stream":       string(stream),
		"stream_id":    handle.StreamID(),
		"state":        string(handle.State()),
		"resume_token": resumeToken,
		"metadata":     metadata,
	})
}

func projectRuntimeDeviceEventPage(request EventsDeviceEventListRequest, outputJSON []byte) ([]byte, error) {
	var output map[string]any
	if err := json.Unmarshal(outputJSON, &output); err != nil {
		return nil, invalidProfilePayload(eventsProfile, fmt.Sprintf("decode events device history output: %v", err), err)
	}
	if output == nil {
		return nil, invalidProfilePayload(eventsProfile, "events device history output must be an object", nil)
	}
	requestedDeviceURA := strings.TrimSpace(request.DeviceURA)
	if output["profile"] == eventsProfile && output["stream"] == string(EventStreamDevice) && output["item_kind"] == "device_event" {
		page, err := NewDeviceEventPageFromJSON(outputJSON)
		if err != nil {
			return nil, err
		}
		if err := validateRuntimeDeviceEventPageMatchesRequest(page, requestedDeviceURA); err != nil {
			return nil, err
		}
		return outputJSON, nil
	}
	rows, err := eventsRuntimeEventRows(output)
	if err != nil {
		return nil, err
	}
	offset, err := eventsRuntimeCursorOffset(request.Cursor)
	if err != nil {
		return nil, err
	}
	if offset > len(rows) {
		return nil, invalidProfilePayload(eventsProfile, "cursor must not point past the current event snapshot", nil)
	}
	end := offset + request.Limit
	if end > len(rows) {
		end = len(rows)
	}
	items := make([]map[string]any, 0, end-offset)
	for _, row := range rows[offset:end] {
		item, err := projectRuntimeDeviceEventRow(row)
		if err != nil {
			return nil, err
		}
		if err := validateRuntimeDeviceEventMatchesRequest(item, requestedDeviceURA); err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	var nextCursor *string
	if end < len(rows) {
		cursor := fmt.Sprintf("%s:%d", EventStreamDevice, end)
		nextCursor = &cursor
	}
	page := map[string]any{
		"profile":     eventsProfile,
		"stream":      string(EventStreamDevice),
		"item_kind":   "device_event",
		"items":       items,
		"next_cursor": nextCursor,
		"has_more":    nextCursor != nil,
		"limit":       request.Limit,
		"metadata": map[string]any{
			"profile":        eventsProfile,
			"source":         "device_event_history",
			"source_ability": eventsAbilityDeviceHistory,
			"total_items":    len(rows),
		},
	}
	raw, err := json.Marshal(page)
	if err != nil {
		return nil, invalidProfilePayload(eventsProfile, fmt.Sprintf("encode events device event page: %v", err), err)
	}
	if _, err := NewDeviceEventPageFromJSON(raw); err != nil {
		return nil, err
	}
	return raw, nil
}

func validateRuntimeDeviceEventPageMatchesRequest(page DeviceEventPage, requestedDeviceURA string) error {
	if requestedDeviceURA == "" {
		return nil
	}
	for index := range page.Items {
		actual := deviceURAFromSubjectRef(page.Items[index].SubjectRef)
		if actual == "" {
			return invalidProfilePayload(eventsProfile, "device event page item subject_ref device URA is required", nil)
		}
		if actual != requestedDeviceURA {
			return invalidProfilePayload(eventsProfile, "device event page item does not match requested device_ura", nil)
		}
	}
	return nil
}

func validateRuntimeDeviceEventMatchesRequest(item map[string]any, requestedDeviceURA string) error {
	if requestedDeviceURA == "" {
		return nil
	}
	actual := deviceURAFromSubjectRef(item["subject_ref"])
	if actual == "" {
		return invalidProfilePayload(eventsProfile, "device event row subject_ref device URA is required", nil)
	}
	if actual != requestedDeviceURA {
		return invalidProfilePayload(eventsProfile, "device event row does not match requested device_ura", nil)
	}
	return nil
}

func deviceURAFromSubjectRef(subjectRef any) string {
	ref, ok := subjectRef.(map[string]any)
	if !ok {
		return ""
	}
	kind, _ := ref["kind"].(string)
	role, _ := ref["role"].(string)
	ura, _ := ref["ura"].(string)
	if strings.TrimSpace(kind) != "ura" || strings.TrimSpace(role) != "device" {
		return ""
	}
	return strings.TrimSpace(ura)
}

func eventsRuntimeEventRows(output map[string]any) ([]map[string]any, error) {
	rawRows, ok := output["events"].([]any)
	if !ok {
		rawRows, ok = output["items"].([]any)
	}
	if !ok {
		return nil, invalidProfilePayload(eventsProfile, "events device history output events must be an array", nil)
	}
	rows := make([]map[string]any, 0, len(rawRows))
	for index, raw := range rawRows {
		row, ok := raw.(map[string]any)
		if !ok {
			return nil, invalidProfilePayload(eventsProfile, fmt.Sprintf("events device history row %d must be an object", index), nil)
		}
		rows = append(rows, row)
	}
	return rows, nil
}

func eventsRuntimeCursorOffset(cursor string) (int, error) {
	if strings.TrimSpace(cursor) == "" {
		return 0, nil
	}
	stream, sequence, ok := parseEventRuntimeResumeToken(cursor)
	if !ok || stream != string(EventStreamDevice) {
		return 0, invalidProfilePayload(eventsProfile, "cursor must use device:<offset> form", nil)
	}
	if sequence > uint64(int(^uint(0)>>1)) {
		return 0, invalidProfilePayload(eventsProfile, "cursor offset is too large", nil)
	}
	return int(sequence), nil
}

func projectRuntimeDeviceEventRow(row map[string]any) (map[string]any, error) {
	if row["profile"] == eventsProfile {
		stream, _ := row["stream"].(string)
		if stream != string(EventStreamDevice) {
			return nil, invalidProfilePayload(eventsProfile, "device event row stream must be device", nil)
		}
		return row, nil
	}
	sequence, ok := numericUint64(row["sequence"])
	if !ok {
		return nil, invalidProfilePayload(eventsProfile, "device event row sequence is required", nil)
	}
	deviceURA, _ := row["device_ura"].(string)
	if strings.TrimSpace(deviceURA) == "" {
		return nil, invalidProfilePayload(eventsProfile, "device event row device_ura is required", nil)
	}
	occurredUnixMS, ok := numericInt64(row["occurred_unix_ms"])
	if !ok || occurredUnixMS < 0 {
		return nil, invalidProfilePayload(eventsProfile, "device event row occurred_unix_ms is required", nil)
	}
	kind, _ := row["kind"].(string)
	if strings.TrimSpace(kind) == "" {
		kind = "device.event"
	}
	payload := row["payload"]
	if payload == nil {
		payload = map[string]any{}
	}
	cursor := map[string]any{
		"stream":   string(EventStreamDevice),
		"sequence": sequence,
		"token":    fmt.Sprintf("%s:%d", EventStreamDevice, sequence),
	}
	return map[string]any{
		"profile":            eventsProfile,
		"stream":             string(EventStreamDevice),
		"kind":               kind,
		"event_id":           firstNonEmpty(firstStringFromMap(row, "event_id"), fmt.Sprintf("evt-%s-%d", EventStreamDevice, sequence)),
		"cursor":             cursor,
		"resume_token":       firstNonEmpty(firstStringFromMap(row, "resume_token"), fmt.Sprintf("%s:%d", EventStreamDevice, sequence)),
		"occurred_unix_ms":   occurredUnixMS,
		"occurred_at":        time.UnixMilli(occurredUnixMS).UTC().Format("2006-01-02T15:04:05.000Z"),
		"subject_ref":        eventsRuntimeDeviceSubjectRef(deviceURA),
		"tenant_ref":         eventsRuntimeTenantRef(row),
		"payload":            payload,
		"dropped_count":      0,
		"reconnect_after_ms": nil,
		"terminal":           false,
		"metadata": map[string]any{
			"profile":        eventsProfile,
			"stream":         string(EventStreamDevice),
			"carrier_owner":  "daemon_sdk",
			"source":         "daemon_device_event",
			"stream_ability": eventsAbilityDeviceHistory,
			"lifecycle":      "history",
		},
	}, nil
}

func eventsRuntimeDeviceSubjectRef(deviceURA string) map[string]any {
	return map[string]any{
		"kind": "ura",
		"ura":  strings.TrimSpace(deviceURA),
		"role": "device",
	}
}

func eventsRuntimeTenantRef(row map[string]any) any {
	if tenantRef, ok := row["tenant_ref"]; ok && tenantRef != nil {
		return tenantRef
	}
	realm, _ := row["realm"].(string)
	if strings.TrimSpace(realm) == "" {
		realm, _ = row["tenant"].(string)
	}
	if strings.TrimSpace(realm) == "" {
		return nil
	}
	return map[string]any{"kind": "realm", "realm": strings.TrimSpace(realm)}
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

func numericInt64(value any) (int64, bool) {
	switch typed := value.(type) {
	case float64:
		if typed != float64(int64(typed)) {
			return 0, false
		}
		return int64(typed), true
	case int:
		return int64(typed), true
	case int64:
		return typed, true
	case uint64:
		if typed > uint64(^uint64(0)>>1) {
			return 0, false
		}
		return int64(typed), true
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

func eventsInvocationFailureError(result InvocationResult) error {
	failure := result.Failure()
	message := "events invocation failed"
	code := ErrAdmissionDenied
	stage := "runtime"
	retry := RetryNever
	details := map[string]any{"terminal_state": result.TerminalState()}
	if failure != nil {
		if failure.Message() != "" {
			message = failure.Message()
		}
		if failure.Code() != "" {
			code = runtimeFailureCode(failure.Code(), ErrAdmissionDenied)
			details["runtime_code"] = failure.Code()
		}
		if failure.Stage() != "" {
			stage = failure.Stage()
		}
		if failure.Retryable() {
			retry = RetrySafe
		}
		details["runtime_retryable"] = failure.Retryable()
	}
	return withProfileErrorDetails(&SDKError{
		Code:      code,
		Stage:     stage,
		Retry:     retry,
		Retryable: RetryableForHint(retry),
		Message:   message,
		Details:   details,
	}, eventsProfile)
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
