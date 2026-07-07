package easynet

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"sync"
)

const (
	directoryAbilityNamespaceResolve     = "namespace.resolve"
	directoryAbilityProxyResolve         = "namespace.proxy_resolve"
	directoryAbilityNodeList             = "node.list"
	directoryAbilityProxyListUserDevices = "federation.proxy_list_user_devices"
	directoryAbilityAgentList            = "agent.list"
	directoryAbilityMetaListAbilities    = "meta.list_abilities"
	directoryAbilitySubscribeDirectory   = "directory.subscribe"
	directoryResolveTypeDirectoryListing = "RESOLVE_TYPE_DIRECTORY_LISTING"
)

var directoryCarrierArgKeys = map[string]struct{}{
	"caller_ura":         {},
	"callee_ura":         {},
	"subject_ura":        {},
	"descriptor_version": {},
	"nonce_base64":       {},
	"causal_context":     {},
	"metadata":           {},
	"limit":              {},
	"cursor":             {},
}

// DirectoryRuntimeTransport lowers Directory profile requests into Runtime
// Core invocations and projects daemon read-model facts back into Directory DTOs.
type DirectoryRuntimeTransport struct {
	runtime  *RuntimeClient
	identity *IdentityClient
	mu       sync.Mutex
	streams  map[string]*StreamHandle
}

func NewDirectoryRuntimeTransport(runtime *RuntimeClient, identity *IdentityClient) (*DirectoryRuntimeTransport, error) {
	if runtime == nil {
		return nil, invalidProfileClient(directoryIdentityProfile, "runtime client is required")
	}
	if identity == nil {
		return nil, invalidProfileClient(directoryIdentityProfile, "identity client is required")
	}
	return &DirectoryRuntimeTransport{
		runtime:  runtime,
		identity: identity,
		streams:  map[string]*StreamHandle{},
	}, nil
}

func NewRuntimeDirectoryClient(runtime *RuntimeClient, identity *IdentityClient) (*DirectoryClient, error) {
	transport, err := NewDirectoryRuntimeTransport(runtime, identity)
	if err != nil {
		return nil, err
	}
	return NewDirectoryClient(transport)
}

func (t *DirectoryRuntimeTransport) BuildDirectorySubscriptionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, _, err := decodeDirectorySubscriptionForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, requestJSON, request.DirectoryQueryBase, directoryAbilitySubscribeDirectory, request)
}

func (t *DirectoryRuntimeTransport) Resolve(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, payload, err := decodeDirectoryResolveForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	abilityName := directoryAbilityNamespaceResolve
	if len(request.PeerHubURLs) > 0 {
		abilityName = directoryAbilityProxyResolve
	}
	output, err := t.invoke(ctx, requestJSON, request.DirectoryQueryBase, abilityName, directoryResolveArgs(payload))
	if err != nil {
		return nil, err
	}
	return projectDirectoryResolvedRef(output, abilityName)
}

func (t *DirectoryRuntimeTransport) ListDevices(ctx context.Context, requestJSON []byte) ([]byte, error) {
	base, _, err := decodeDirectoryPageForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, requestJSON, base, directoryAbilityNodeList, map[string]any{})
	if err != nil {
		return nil, err
	}
	return projectDirectoryDevicePage(output, base)
}

func (t *DirectoryRuntimeTransport) ListPeerUserDevices(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodePeerUserDeviceQueryForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(
		ctx,
		requestJSON,
		request.DirectoryQueryBase,
		directoryAbilityProxyListUserDevices,
		map[string]any{
			"realm":         request.UserTenantID,
			"peer_hub_urls": request.PeerHubURLs,
		},
	)
	if err != nil {
		return nil, err
	}
	return projectDirectoryPeerUserDevicePage(output, request.DirectoryQueryBase)
}

func (t *DirectoryRuntimeTransport) ListAgents(ctx context.Context, requestJSON []byte) ([]byte, error) {
	base, _, err := decodeDirectoryPageForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, requestJSON, base, directoryAbilityAgentList, map[string]any{})
	if err != nil {
		return nil, err
	}
	return projectDirectoryAgentPage(output, base)
}

func (t *DirectoryRuntimeTransport) ListAbilities(ctx context.Context, requestJSON []byte) ([]byte, error) {
	var request AbilityQuery
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode directory ability query: %v", err), err)
	}
	request.DirectoryQueryBase = normalizeDirectoryPageQuery(request.DirectoryQueryBase)
	if err := validateDirectoryQueryBase(request.DirectoryQueryBase, true); err != nil {
		return nil, err
	}
	payload, err := directoryPayloadObject(requestJSON)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, requestJSON, request.DirectoryQueryBase, directoryAbilityMetaListAbilities, directoryAbilityArgs(payload))
	if err != nil {
		return nil, err
	}
	return projectDirectoryAbilityPage(output, request.DirectoryQueryBase)
}

func (t *DirectoryRuntimeTransport) SubscribeDirectory(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, _, err := decodeDirectorySubscriptionForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	draft, err := t.buildInvocation(ctx, request.DirectoryQueryBase, directoryAbilitySubscribeDirectory, request)
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
	return directoryRuntimeSubscriptionOpenJSON(request, handle)
}

func (t *DirectoryRuntimeTransport) Close(ctx context.Context) error {
	if ctx == nil {
		return invalidProfileClient(directoryIdentityProfile, "context is required")
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

func (t *DirectoryRuntimeTransport) buildInvocationJSON(ctx context.Context, requestJSON []byte, base DirectoryQueryBase, abilityName string, args any) ([]byte, error) {
	draft, err := t.buildInvocation(ctx, base, abilityName, args)
	if err != nil {
		return nil, err
	}
	raw, err := json.Marshal(draft)
	if err != nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("encode directory invocation: %v", err), err)
	}
	return raw, nil
}

func (t *DirectoryRuntimeTransport) buildInvocation(ctx context.Context, base DirectoryQueryBase, abilityName string, args any) (InvocationDraft, error) {
	if t == nil || t.runtime == nil || t.identity == nil {
		return InvocationDraft{}, invalidProfileClient(directoryIdentityProfile, "directory runtime transport is not initialized")
	}
	if ctx == nil {
		return InvocationDraft{}, invalidProfileClient(directoryIdentityProfile, "context is required")
	}
	descriptorRef, err := t.identity.OwnerAbilityDescriptorRef(ctx, base.CalleeURA, abilityName, base.DescriptorVersion)
	if err != nil {
		return InvocationDraft{}, err
	}
	return NewInvocationBuilder().
		WithCallerURA(base.CallerURA).
		WithCalleeURA(base.CalleeURA).
		WithDescriptorRef(descriptorRef).
		WithSubjectURA(base.SubjectURA).
		WithNonceBase64(base.NonceBase64).
		WithCausalContext(base.CausalContext).
		WithJSONArgs(args).
		WithContentType("application/json").
		WithMetadata(directoryRuntimeMetadata(base.Metadata, abilityName)).
		Build()
}

func (t *DirectoryRuntimeTransport) invoke(ctx context.Context, requestJSON []byte, base DirectoryQueryBase, abilityName string, args any) ([]byte, error) {
	draft, err := t.buildInvocation(ctx, base, abilityName, args)
	if err != nil {
		return nil, err
	}
	result, err := t.runtime.Invoke(ctx, draft)
	if err != nil {
		return nil, err
	}
	if !result.OK() {
		return nil, directoryInvocationFailureError(result)
	}
	outputJSON := result.OutputJSON()
	if len(outputJSON) == 0 || string(outputJSON) == "null" {
		return nil, invalidProfilePayload(directoryIdentityProfile, "directory invocation output_json is required", nil)
	}
	return outputJSON, nil
}

func (t *DirectoryRuntimeTransport) bindDirectorySubscriptionHandle(subscription DirectorySubscription) DirectorySubscription {
	if t == nil {
		return subscription
	}
	streamID := subscription.MetadataStreamID()
	if streamID == "" {
		return subscription
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	subscription.handle = t.streams[streamID]
	subscription.release = t.releaseDirectorySubscriptionHandle
	return subscription
}

func (t *DirectoryRuntimeTransport) releaseDirectorySubscriptionHandle(streamID string) {
	if t == nil || streamID == "" {
		return
	}
	t.mu.Lock()
	delete(t.streams, streamID)
	t.mu.Unlock()
}

func directoryRuntimeSubscriptionOpenJSON(request DirectorySubscriptionRequest, handle *StreamHandle) ([]byte, error) {
	if handle == nil {
		return nil, invalidProfileClient(directoryIdentityProfile, "runtime stream handle is required")
	}
	cursor := DirectorySubscriptionCursor{
		Stream:   directorySubscriptionStream,
		Sequence: 0,
		Token:    fmt.Sprintf("%s:%d", directorySubscriptionStream, 0),
	}
	if request.ResumeCursor != nil {
		cursor = *request.ResumeCursor
	}
	metadata := directoryRuntimeMetadata(request.Metadata, directoryAbilitySubscribeDirectory)
	metadata["source"] = "runtime_stream"
	metadata["runtime_stream_id"] = handle.StreamID()
	metadata["max_buffered_events"] = handle.MaxBufferedEvents()
	return json.Marshal(map[string]any{
		"profile":      directoryIdentityProfile,
		"kind":         "directory_subscription",
		"stream":       directorySubscriptionStream,
		"state":        DirectorySubscriptionOpening,
		"cursor":       cursor,
		"resume_token": cursor.ResumeToken(),
		"events":       []any{},
		"drop_count":   0,
		"metadata":     metadata,
	})
}

func decodeDirectoryResolveForRuntime(requestJSON []byte) (ResolveQuery, map[string]any, error) {
	var request ResolveQuery
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		return ResolveQuery{}, nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode directory resolve request: %v", err), err)
	}
	if err := validateDirectoryQueryBase(request.DirectoryQueryBase, false); err != nil {
		return ResolveQuery{}, nil, err
	}
	if strings.TrimSpace(request.QueryName) == "" && strings.TrimSpace(request.RealmHint) == "" {
		return ResolveQuery{}, nil, invalidProfilePayload(directoryIdentityProfile, "query_name or realm_hint is required", nil)
	}
	if err := validatePeerHubURLs(request.PeerHubURLs, false); err != nil {
		return ResolveQuery{}, nil, err
	}
	payload, err := directoryPayloadObject(requestJSON)
	if err != nil {
		return ResolveQuery{}, nil, err
	}
	return request, payload, nil
}

func decodeDirectoryPageForRuntime(requestJSON []byte) (DirectoryQueryBase, map[string]any, error) {
	var base DirectoryQueryBase
	if err := json.Unmarshal(requestJSON, &base); err != nil {
		return DirectoryQueryBase{}, nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode directory page request: %v", err), err)
	}
	base = normalizeDirectoryPageQuery(base)
	if err := validateDirectoryQueryBase(base, true); err != nil {
		return DirectoryQueryBase{}, nil, err
	}
	payload, err := directoryPayloadObject(requestJSON)
	if err != nil {
		return DirectoryQueryBase{}, nil, err
	}
	return base, payload, nil
}

func decodePeerUserDeviceQueryForRuntime(requestJSON []byte) (PeerUserDeviceQuery, error) {
	var request PeerUserDeviceQuery
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		return PeerUserDeviceQuery{}, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode peer user-device request: %v", err), err)
	}
	request.DirectoryQueryBase = normalizeDirectoryPageQuery(request.DirectoryQueryBase)
	if err := validatePeerUserDeviceQuery(request); err != nil {
		return PeerUserDeviceQuery{}, err
	}
	return request, nil
}

func decodeDirectorySubscriptionForRuntime(requestJSON []byte) (DirectorySubscriptionRequest, map[string]any, error) {
	var request DirectorySubscriptionRequest
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		return DirectorySubscriptionRequest{}, nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode directory subscription request: %v", err), err)
	}
	normalized, err := normalizeDirectorySubscriptionRequest(request)
	if err != nil {
		return DirectorySubscriptionRequest{}, nil, err
	}
	payload, err := directoryPayloadObject(requestJSON)
	if err != nil {
		return DirectorySubscriptionRequest{}, nil, err
	}
	return normalized, payload, nil
}

func directoryPayloadObject(requestJSON []byte) (map[string]any, error) {
	var payload map[string]any
	if err := json.Unmarshal(requestJSON, &payload); err != nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode directory request: %v", err), err)
	}
	if payload == nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, "directory request must be an object", nil)
	}
	return payload, nil
}

func directoryResolveArgs(payload map[string]any) map[string]any {
	args := map[string]any{}
	if value := stringArg(payload, "query_name"); value != "" {
		args["query_name"] = value
	}
	if value := stringArg(payload, "ability_name"); value != "" {
		args["ability_name"] = value
	}
	if value := normalizeDirectoryResolveType(stringArg(payload, "qtype")); value != "" {
		args["qtype"] = value
	}
	if value := stringArg(payload, "realm_hint"); value != "" {
		args["realm_hint"] = value
	}
	if value, ok := payload["peer_hub_urls"]; ok {
		args["peer_hub_urls"] = value
	}
	return args
}

func directoryAbilityArgs(payload map[string]any) map[string]any {
	args := map[string]any{}
	if value := stringArg(payload, "scope"); value != "" {
		args["scope"] = value
	}
	if value := stringArg(payload, "owner_ura"); value != "" {
		args["agent_ura"] = value
	}
	if value := stringArg(payload, "ability_ura"); value != "" {
		args["subject_ura"] = value
	}
	return args
}

func normalizeDirectoryResolveType(value string) string {
	value = strings.TrimSpace(value)
	if value == "" {
		return directoryResolveTypeDirectoryListing
	}
	value = strings.ToUpper(strings.ReplaceAll(strings.TrimPrefix(value, "RESOLVE_TYPE_"), "-", "_"))
	switch value {
	case "CANONICAL_IDENTITY", "OWNER", "ABILITY", "ROUTE", "KEY", "SERVICE", "DIRECTORY_LISTING":
		return "RESOLVE_TYPE_" + value
	default:
		return value
	}
}

func directoryRuntimeMetadata(input map[string]any, abilityName string) map[string]any {
	metadata := map[string]any{}
	for key, value := range input {
		metadata[key] = value
	}
	metadata["profile"] = directoryIdentityProfile
	metadata["system_ability"] = abilityName
	metadata["carrier_owner"] = "daemon_sdk"
	return metadata
}

func projectDirectoryResolvedRef(raw []byte, sourceAbility string) ([]byte, error) {
	payload, err := directoryOutputObject(raw)
	if err != nil {
		return nil, err
	}
	answer := payload
	if nested, ok := payload["answer"].(map[string]any); ok {
		answer = nested
	}
	answerKind := firstStringFromMap(answer, "answer_kind")
	if answerKind == "" && firstValue(answer, "negative") != nil {
		answerKind = "RESOLVE_ANSWER_KIND_NEGATIVE"
	}
	if answerKind == "" {
		return nil, invalidProfilePayload(directoryIdentityProfile, "directory resolve answer_kind is required", nil)
	}
	records, ok := answer["records"].([]any)
	if !ok {
		records = []any{}
	}
	result := map[string]any{
		"profile":          directoryIdentityProfile,
		"kind":             "resolved_ref",
		"answer_kind":      answerKind,
		"query_name":       firstStringPtr(firstStringFromMap(answer, "query_name")),
		"canonical_name":   firstStringPtr(firstStringFromMap(answer, "canonical_name")),
		"owner_ura":        firstStringPtr(firstStringFromMap(answer, "owner_ura")),
		"ability_ura":      firstStringPtr(firstStringFromMap(answer, "ability_ura")),
		"route_ura":        firstStringPtr(firstStringFromMap(answer, "route_ura")),
		"next_hop":         firstValue(answer, "next_hop"),
		"selected_route":   firstValue(answer, "selected_route"),
		"route_candidates": firstNonNilValue(answer, []any{}, "route_candidates"),
		"records":          records,
		"negative":         firstValue(answer, "negative"),
		"release_profile":  firstStringPtr(firstStringFromMap(answer, "release_profile")),
		"authority":        firstValue(answer, "authority"),
		"cache_policy":     firstValue(answer, "cache_policy"),
		"metadata": map[string]any{
			"profile":    directoryIdentityProfile,
			"source":     sourceAbility,
			"raw_answer": answer,
		},
	}
	if result["query_name"] == nil {
		result["query_name"] = result["canonical_name"]
	}
	return json.Marshal(result)
}

func projectDirectoryDevicePage(raw []byte, base DirectoryQueryBase) ([]byte, error) {
	payload, err := directoryOutputObject(raw)
	if err != nil {
		return nil, err
	}
	rows := firstArray(payload, "nodes", "devices", "items")
	items := make([]any, 0, len(rows))
	for _, row := range rows {
		obj, ok := row.(map[string]any)
		if !ok {
			return nil, invalidProfilePayload(directoryIdentityProfile, "device row must be an object", nil)
		}
		nodeID := firstStringFromMap(obj, "node_id")
		deviceURA := firstStringFromMap(obj, "device_ura")
		if deviceURA == "" {
			return nil, invalidProfilePayload(directoryIdentityProfile, "device row missing device_ura", nil)
		}
		state := firstNonEmpty(firstStringFromMap(obj, "state"), "unknown")
		items = append(items, map[string]any{
			"profile":      directoryIdentityProfile,
			"kind":         "device",
			"node_id":      nodeID,
			"device_ura":   deviceURA,
			"state":        state,
			"online":       state == "online" || state == "active",
			"is_self":      boolArg(obj, "is_self"),
			"paired":       boolArg(obj, "paired"),
			"tenant_id":    firstStringFromMap(obj, "tenant_id", "tenant", "realm"),
			"hub_endpoint": firstStringFromMap(obj, "hub_endpoint"),
			"probe_status": firstStringFromMap(obj, "probe_status"),
			"probe_error":  firstValue(obj, "probe_error"),
			"latency_ms":   numberArg(obj, "latency_ms"),
			"abilities":    firstNonNilValue(obj, []any{}, "abilities"),
			"metadata":     metadataWithSource(obj, directoryAbilityNodeList),
		})
	}
	return directoryPageJSON("device_page", "device", directoryAbilityNodeList, base.Limit, items)
}

func projectDirectoryPeerUserDevicePage(raw []byte, base DirectoryQueryBase) ([]byte, error) {
	payload, err := directoryOutputObject(raw)
	if err != nil {
		return nil, err
	}
	rows := firstArray(payload, "devices", "items")
	items := make([]any, 0, len(rows))
	for _, row := range rows {
		obj, ok := row.(map[string]any)
		if !ok {
			return nil, invalidProfilePayload(directoryIdentityProfile, "peer user-device row must be an object", nil)
		}
		agentURA := firstStringFromMap(obj, "agent_ura")
		if agentURA == "" {
			return nil, invalidProfilePayload(directoryIdentityProfile, "peer user-device row missing agent_ura", nil)
		}
		nodeID := firstStringFromMap(obj, "node_id")
		status := firstNonEmpty(firstStringFromMap(obj, "status"), "unknown")
		items = append(items, map[string]any{
			"profile":           directoryIdentityProfile,
			"kind":              "peer_user_device",
			"agent_ura":         agentURA,
			"node_id":           nodeID,
			"display_name":      firstStringFromMap(obj, "display_name"),
			"status":            status,
			"origin_realm":      firstStringFromMap(obj, "origin_realm"),
			"hub_endpoint":      firstStringFromMap(obj, "hub_endpoint"),
			"last_seen_unix_ms": numberArgInt64(obj, "last_seen_unix_ms"),
			"metadata":          metadataWithSource(obj, directoryAbilityProxyListUserDevices),
		})
	}
	return directoryPageJSON("peer_user_device_page", "peer_user_device", directoryAbilityProxyListUserDevices, base.Limit, items)
}

func projectDirectoryAgentPage(raw []byte, base DirectoryQueryBase) ([]byte, error) {
	payload, err := directoryOutputObject(raw)
	if err != nil {
		return nil, err
	}
	rows := firstArray(payload, "agents", "items")
	items := make([]any, 0, len(rows))
	for _, row := range rows {
		obj, ok := row.(map[string]any)
		if !ok {
			return nil, invalidProfilePayload(directoryIdentityProfile, "agent row must be an object", nil)
		}
		agentURA := firstStringFromMap(obj, "agent_ura")
		if agentURA == "" {
			return nil, invalidProfilePayload(directoryIdentityProfile, "agent row missing agent_ura", nil)
		}
		items = append(items, map[string]any{
			"name":       firstStringFromMap(obj, "name"),
			"agent_ura":  firstStringPtr(agentURA),
			"owner_ura":  firstStringPtr(firstStringFromMap(obj, "owner_ura")),
			"device_ura": firstStringPtr(firstStringFromMap(obj, "device_ura")),
			"state":      firstNonEmpty(firstStringFromMap(obj, "state"), "unknown"),
			"runtime":    firstNonEmpty(firstStringFromMap(obj, "runtime"), "daemon"),
			"model":      firstStringPtr(firstStringFromMap(obj, "model")),
			"label":      firstStringPtr(firstStringFromMap(obj, "label")),
			"abilities":  firstNonNilValue(obj, []any{}, "abilities"),
			"metadata":   metadataWithSource(obj, directoryAbilityAgentList),
		})
	}
	return directoryPageJSON("agent_page", "agent", directoryAbilityAgentList, base.Limit, items)
}

func projectDirectoryAbilityPage(raw []byte, base DirectoryQueryBase) ([]byte, error) {
	payload, err := directoryOutputObject(raw)
	if err != nil {
		return nil, err
	}
	rows := firstArray(payload, "abilities", "items")
	items := make([]any, 0, len(rows))
	for _, row := range rows {
		obj, ok := row.(map[string]any)
		if !ok {
			return nil, invalidProfilePayload(directoryIdentityProfile, "ability row must be an object", nil)
		}
		items = append(items, map[string]any{
			"profile":            directoryIdentityProfile,
			"kind":               "ability",
			"name":               firstStringFromMap(obj, "name"),
			"ability_ura":        firstStringFromMap(obj, "ability_ura"),
			"owner_ura":          firstStringFromMap(obj, "owner_ura"),
			"descriptor_ref":     firstStringPtr(firstStringFromMap(obj, "descriptor_ref")),
			"descriptor_version": firstNonEmpty(firstStringFromMap(obj, "descriptor_version"), "1.0.0"),
			"visibility":         firstStringFromMap(obj, "visibility"),
			"class":              firstStringFromMap(obj, "class"),
			"description":        firstStringFromMap(obj, "description"),
			"source":             firstStringFromMap(obj, "source"),
			"schema_summary":     firstNonNilValue(obj, map[string]any{}, "schema_summary"),
			"hints":              firstNonNilValue(obj, map[string]any{}, "hints"),
			"metadata":           metadataWithSource(obj, directoryAbilityMetaListAbilities),
		})
	}
	return directoryPageJSON("ability_page", "ability", directoryAbilityMetaListAbilities, base.Limit, items)
}

func directoryPageJSON(kind string, itemKind string, sourceAbility string, limit int, items []any) ([]byte, error) {
	return json.Marshal(map[string]any{
		"profile":     directoryIdentityProfile,
		"kind":        kind,
		"item_kind":   itemKind,
		"items":       items,
		"next_cursor": nil,
		"limit":       limit,
		"source":      directoryReadModelSource,
		"metadata": map[string]any{
			"profile":        directoryIdentityProfile,
			"source":         directoryReadModelSource,
			"source_ability": sourceAbility,
			"count":          len(items),
		},
	})
}

func directoryOutputObject(raw []byte) (map[string]any, error) {
	var payload map[string]any
	if err := json.Unmarshal(raw, &payload); err != nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode directory output: %v", err), err)
	}
	if payload == nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, "directory output must be an object", nil)
	}
	if nested, ok := payload["result"].(map[string]any); ok {
		return nested, nil
	}
	return payload, nil
}

func directoryInvocationFailureError(result InvocationResult) error {
	failure := result.Failure()
	message := "directory invocation failed"
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
	}, directoryIdentityProfile)
}

func stringArg(values map[string]any, key string) string {
	value, _ := values[key].(string)
	return strings.TrimSpace(value)
}

func boolArg(values map[string]any, key string) bool {
	value, _ := values[key].(bool)
	return value
}

func numberArg(values map[string]any, keys ...string) int {
	for _, key := range keys {
		switch value := values[key].(type) {
		case float64:
			return int(value)
		case int:
			return value
		}
	}
	return 0
}

func numberArgInt64(values map[string]any, keys ...string) int64 {
	for _, key := range keys {
		switch value := values[key].(type) {
		case float64:
			return int64(value)
		case int64:
			return value
		case int:
			return int64(value)
		}
	}
	return 0
}

func firstStringFromMap(values map[string]any, keys ...string) string {
	for _, key := range keys {
		if value, ok := values[key].(string); ok && strings.TrimSpace(value) != "" {
			return strings.TrimSpace(value)
		}
	}
	return ""
}

func firstStringPtr(value string) *string {
	if strings.TrimSpace(value) == "" {
		return nil
	}
	trimmed := strings.TrimSpace(value)
	return &trimmed
}

func firstValue(values map[string]any, keys ...string) any {
	for _, key := range keys {
		if value, ok := values[key]; ok {
			return value
		}
	}
	return nil
}

func firstNonNilValue(values map[string]any, fallback any, keys ...string) any {
	for _, key := range keys {
		if value, ok := values[key]; ok && value != nil {
			return value
		}
	}
	return fallback
}

func firstArray(values map[string]any, keys ...string) []any {
	for _, key := range keys {
		if array, ok := values[key].([]any); ok {
			return array
		}
	}
	return []any{}
}

func metadataWithSource(row map[string]any, source string) map[string]any {
	metadata, _ := row["metadata"].(map[string]any)
	out := map[string]any{}
	for key, value := range metadata {
		out[key] = value
	}
	out["profile"] = directoryIdentityProfile
	out["source"] = source
	return out
}
