package easynet

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
)

const (
	surfaceAbilityListPages  = "pages.list"
	surfaceAbilityCreatePage = "pages.publish"
	surfaceAbilityDeletePage = "pages.unpublish"
	surfaceAbilityManifest   = "pages.get"
	surfaceAbilityHealth     = "pages.health"
)

// SurfaceRuntimeTransport lowers Surface requests into daemon Runtime
// invocations and projects daemon pages.* facts back into Surface DTOs.
type SurfaceRuntimeTransport struct {
	runtime  *RuntimeClient
	identity *IdentityClient
}

func NewSurfaceRuntimeTransport(runtime *RuntimeClient, identity *IdentityClient) (*SurfaceRuntimeTransport, error) {
	if runtime == nil {
		return nil, invalidProfileClient(surfaceProfile, "runtime client is required")
	}
	if identity == nil {
		return nil, invalidProfileClient(surfaceProfile, "identity client is required")
	}
	return &SurfaceRuntimeTransport{runtime: runtime, identity: identity}, nil
}

func NewRuntimeSurfaceClient(runtime *RuntimeClient, identity *IdentityClient) (*SurfaceClient, error) {
	transport, err := NewSurfaceRuntimeTransport(runtime, identity)
	if err != nil {
		return nil, err
	}
	return NewSurfaceClient(transport)
}

func (t *SurfaceRuntimeTransport) BuildListPagesInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeSurfaceListPagesForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, requestJSON, request.SurfaceCarrierBase, surfaceAbilityListPages)
}

func (t *SurfaceRuntimeTransport) BuildCreatePageInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeSurfaceCreatePageForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, requestJSON, request.SurfaceCarrierBase, surfaceAbilityCreatePage)
}

func (t *SurfaceRuntimeTransport) BuildDeletePageInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeSurfaceDeletePageForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, requestJSON, request.SurfaceCarrierBase, surfaceAbilityDeletePage)
}

func (t *SurfaceRuntimeTransport) BuildManifestInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeSurfaceManifestForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, requestJSON, request.SurfaceCarrierBase, surfaceAbilityManifest)
}

func (t *SurfaceRuntimeTransport) BuildHealthInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeSurfaceHealthForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, requestJSON, request.SurfaceCarrierBase, surfaceAbilityHealth)
}

func (t *SurfaceRuntimeTransport) ListPages(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeSurfaceListPagesForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, result, err := t.invoke(ctx, requestJSON, request.SurfaceCarrierBase, surfaceAbilityListPages)
	if err != nil {
		return nil, err
	}
	return projectSurfacePagePage(output, surfaceProjectionHints{
		OwnerURA:       request.CalleeURA,
		Realm:          realmFromURA(request.CalleeURA),
		Limit:          normalizedSurfaceLimit(request.Limit),
		Cursor:         request.Cursor,
		SelectedNodeID: result.SelectedNodeID(),
	})
}

func (t *SurfaceRuntimeTransport) CreatePage(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeSurfaceCreatePageForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, result, err := t.invoke(ctx, requestJSON, request.SurfaceCarrierBase, surfaceAbilityCreatePage)
	if err != nil {
		return nil, err
	}
	return projectSurfacePageRecord(output, surfaceProjectionHints{
		OwnerURA:       request.CalleeURA,
		Realm:          realmFromURA(request.CalleeURA),
		SelectedNodeID: result.SelectedNodeID(),
	})
}

func (t *SurfaceRuntimeTransport) DeletePage(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeSurfaceDeletePageForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, _, err := t.invoke(ctx, requestJSON, request.SurfaceCarrierBase, surfaceAbilityDeletePage)
	if err != nil {
		return nil, err
	}
	return projectSurfaceMutationResult(output)
}

func (t *SurfaceRuntimeTransport) SurfaceManifest(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeSurfaceManifestForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, result, err := t.invoke(ctx, requestJSON, request.SurfaceCarrierBase, surfaceAbilityManifest)
	if err != nil {
		return nil, err
	}
	return projectSurfaceManifest(output, surfaceProjectionHints{
		OwnerURA:       request.CalleeURA,
		Realm:          realmFromURA(request.CalleeURA),
		SelectedNodeID: result.SelectedNodeID(),
	})
}

func (t *SurfaceRuntimeTransport) PublicPageRef(_ context.Context, requestJSON []byte) ([]byte, error) {
	return projectSurfacePublicPageRef(requestJSON, surfaceProjectionHints{})
}

func (t *SurfaceRuntimeTransport) SurfaceHealth(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeSurfaceHealthForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, _, err := t.invoke(ctx, requestJSON, request.SurfaceCarrierBase, surfaceAbilityHealth)
	if err != nil {
		return nil, err
	}
	return projectSurfaceHealth(output, request.SurfaceCarrierBase)
}

func (t *SurfaceRuntimeTransport) Close(context.Context) error {
	return nil
}

func (t *SurfaceRuntimeTransport) buildInvocationJSON(ctx context.Context, requestJSON []byte, base SurfaceCarrierBase, abilityName string) ([]byte, error) {
	draft, err := t.buildInvocation(ctx, requestJSON, base, abilityName)
	if err != nil {
		return nil, err
	}
	raw, err := json.Marshal(draft)
	if err != nil {
		return nil, invalidProfilePayload(surfaceProfile, fmt.Sprintf("encode surface invocation: %v", err), err)
	}
	return raw, nil
}

func (t *SurfaceRuntimeTransport) buildInvocation(ctx context.Context, requestJSON []byte, base SurfaceCarrierBase, abilityName string) (InvocationDraft, error) {
	if t == nil || t.runtime == nil || t.identity == nil {
		return InvocationDraft{}, invalidProfileClient(surfaceProfile, "surface runtime transport is not initialized")
	}
	if ctx == nil {
		return InvocationDraft{}, invalidProfileClient(surfaceProfile, "context is required")
	}
	payload, err := surfaceRuntimePayload(requestJSON)
	if err != nil {
		return InvocationDraft{}, err
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
		WithJSONArgs(surfaceRuntimeArgs(payload, abilityName)).
		WithContentType("application/json").
		WithMetadata(surfaceRuntimeMetadata(base.Metadata, abilityName)).
		Build()
}

func (t *SurfaceRuntimeTransport) invoke(ctx context.Context, requestJSON []byte, base SurfaceCarrierBase, abilityName string) ([]byte, InvocationResult, error) {
	draft, err := t.buildInvocation(ctx, requestJSON, base, abilityName)
	if err != nil {
		return nil, InvocationResult{}, err
	}
	result, err := t.runtime.Invoke(ctx, draft)
	if err != nil {
		return nil, InvocationResult{}, err
	}
	if !result.OK() {
		return nil, InvocationResult{}, surfaceInvocationFailureError(result)
	}
	outputJSON := result.OutputJSON()
	if len(outputJSON) == 0 || string(outputJSON) == "null" {
		return nil, InvocationResult{}, invalidProfilePayload(surfaceProfile, "surface invocation output_json is required", nil)
	}
	return outputJSON, result, nil
}

func decodeSurfaceListPagesForRuntime(requestJSON []byte) (SurfaceListPagesRequest, error) {
	var req SurfaceListPagesRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return SurfaceListPagesRequest{}, invalidProfilePayload(surfaceProfile, fmt.Sprintf("decode surface list-pages request: %v", err), err)
	}
	if err := validateSurfaceListPagesRequest(req); err != nil {
		return SurfaceListPagesRequest{}, err
	}
	return req, nil
}

func decodeSurfaceCreatePageForRuntime(requestJSON []byte) (SurfaceCreatePageRequest, error) {
	var req SurfaceCreatePageRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return SurfaceCreatePageRequest{}, invalidProfilePayload(surfaceProfile, fmt.Sprintf("decode surface create-page request: %v", err), err)
	}
	if err := validateSurfaceCreatePageRequest(req); err != nil {
		return SurfaceCreatePageRequest{}, err
	}
	return req, nil
}

func decodeSurfaceDeletePageForRuntime(requestJSON []byte) (SurfaceDeletePageRequest, error) {
	var req SurfaceDeletePageRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return SurfaceDeletePageRequest{}, invalidProfilePayload(surfaceProfile, fmt.Sprintf("decode surface delete-page request: %v", err), err)
	}
	if err := validateSurfaceDeletePageRequest(req); err != nil {
		return SurfaceDeletePageRequest{}, err
	}
	return req, nil
}

func decodeSurfaceManifestForRuntime(requestJSON []byte) (SurfaceManifestRequest, error) {
	var req SurfaceManifestRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return SurfaceManifestRequest{}, invalidProfilePayload(surfaceProfile, fmt.Sprintf("decode surface manifest request: %v", err), err)
	}
	if err := validateSurfaceManifestRequest(req); err != nil {
		return SurfaceManifestRequest{}, err
	}
	return req, nil
}

func decodeSurfaceHealthForRuntime(requestJSON []byte) (SurfaceHealthRequest, error) {
	var req SurfaceHealthRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return SurfaceHealthRequest{}, invalidProfilePayload(surfaceProfile, fmt.Sprintf("decode surface health request: %v", err), err)
	}
	if err := validateSurfaceHealthRequest(req); err != nil {
		return SurfaceHealthRequest{}, err
	}
	return req, nil
}

func surfaceRuntimePayload(requestJSON []byte) (map[string]any, error) {
	var payload map[string]any
	if err := json.Unmarshal(requestJSON, &payload); err != nil {
		return nil, invalidProfilePayload(surfaceProfile, fmt.Sprintf("decode surface request: %v", err), err)
	}
	if payload == nil {
		return nil, invalidProfilePayload(surfaceProfile, "surface request must be an object", nil)
	}
	return payload, nil
}

func surfaceRuntimeArgs(payload map[string]any, abilityName string) map[string]any {
	switch abilityName {
	case surfaceAbilityCreatePage:
		args := map[string]any{
			"project_id": payload["project_id"],
			"folder":     payload["folder"],
		}
		if visibility, _ := payload["visibility"].(string); strings.TrimSpace(visibility) != "" {
			args["visibility"] = visibility
		}
		return args
	case surfaceAbilityDeletePage, surfaceAbilityManifest:
		return map[string]any{"project_id": payload["project_id"]}
	case surfaceAbilityHealth:
		args := map[string]any{}
		if projectID, _ := payload["project_id"].(string); strings.TrimSpace(projectID) != "" {
			args["project_id"] = projectID
		}
		if surfaceRef, _ := payload["surface_ref"].(string); strings.TrimSpace(surfaceRef) != "" {
			args["surface_ref"] = surfaceRef
		}
		return args
	default:
		return map[string]any{}
	}
}

func surfaceRuntimeMetadata(input map[string]any, abilityName string) map[string]any {
	metadata := map[string]any{}
	for key, value := range input {
		metadata[key] = value
	}
	metadata["profile"] = surfaceProfile
	metadata["system_ability"] = abilityName
	metadata["carrier_owner"] = "daemon_sdk"
	return metadata
}

func surfaceInvocationFailureError(result InvocationResult) error {
	failure := result.Failure()
	message := "surface invocation failed"
	code := ErrAdmissionDenied
	stage := "runtime"
	retry := RetryNever
	details := map[string]any{"terminal_state": result.TerminalState()}
	if failure != nil {
		if failure.Message() != "" {
			message = failure.Message()
		}
		if failure.Code() != "" {
			code = NormalizeErrorCode(failure.Code())
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
	}, surfaceProfile)
}

type surfaceProjectionHints struct {
	OwnerURA       string
	Realm          string
	Limit          int
	Cursor         string
	SelectedNodeID string
}

func projectSurfacePagePage(raw []byte, hints surfaceProjectionHints) ([]byte, error) {
	payload, err := surfaceOutputObject(raw)
	if err != nil {
		return nil, err
	}
	if payload["profile"] == surfaceProfile && payload["kind"] == "surface_page_page" {
		return raw, nil
	}
	rows, err := surfaceRows(payload)
	if err != nil {
		return nil, err
	}
	start, err := surfaceCursorOffset(hints.Cursor)
	if err != nil {
		return nil, err
	}
	limit := normalizedSurfaceLimit(hints.Limit)
	end := start + limit
	if end > len(rows) {
		end = len(rows)
	}
	if start > len(rows) {
		start = len(rows)
		end = len(rows)
	}
	items := make([]json.RawMessage, 0, end-start)
	for _, row := range rows[start:end] {
		record, err := projectSurfacePageRecord(row, hints)
		if err != nil {
			return nil, err
		}
		items = append(items, json.RawMessage(record))
	}
	var nextCursor *string
	if end < len(rows) {
		cursor := fmt.Sprintf("%d", end)
		nextCursor = &cursor
	}
	return json.Marshal(map[string]any{
		"profile":     surfaceProfile,
		"kind":        "surface_page_page",
		"item_kind":   "page_record",
		"items":       items,
		"next_cursor": nextCursor,
		"limit":       limit,
		"source":      surfaceReadModelSource,
		"metadata": map[string]any{
			"profile":           surfaceProfile,
			"source_ability":    surfaceAbilityListPages,
			"page_size_default": DefaultSurfacePageSize,
			"page_size_max":     MaxSurfacePageSize,
			"total_available":   len(rows),
			"selected_node_id":  hints.SelectedNodeID,
		},
	})
}

func projectSurfacePageRecord(raw []byte, hints surfaceProjectionHints) ([]byte, error) {
	payload, err := surfaceOutputObject(raw)
	if err != nil {
		return nil, err
	}
	if payload["profile"] == surfaceProfile && payload["kind"] == "page_record" {
		return raw, nil
	}
	pageID := firstSurfaceString(payload, "page_id", "project_id", "id")
	if pageID == "" {
		return nil, invalidProfilePayload(surfaceProfile, "surface page_id is required", nil)
	}
	if err := validateSurfaceProjectID(pageID); err != nil {
		return nil, err
	}
	user := firstSurfaceString(payload, "user")
	surfaceRef := firstSurfaceString(payload, "surface_ref", "project_ura", "resource_ura")
	if surfaceRef == "" && user != "" && hints.Realm != "" {
		surfaceRef = fmt.Sprintf("easynet:///r/%s/resource/%s.%s", hints.Realm, user, pageID)
	}
	if surfaceRef == "" {
		return nil, invalidProfilePayload(surfaceProfile, "surface_ref is required", nil)
	}
	ownerURA := firstSurfaceString(payload, "owner_ura")
	if ownerURA == "" {
		ownerURA = hints.OwnerURA
	}
	if ownerURA == "" && user != "" {
		realm := realmFromURA(surfaceRef)
		if realm != "" {
			ownerURA = fmt.Sprintf("easynet:///r/%s/agent/%s.pages", realm, user)
		}
	}
	if ownerURA == "" {
		return nil, invalidProfilePayload(surfaceProfile, "owner_ura is required", nil)
	}
	publicRef := firstSurfaceString(payload, "public_ref", "url_root", "public_url")
	var publicRefPtr *string
	if publicRef != "" {
		if err := validateSurfacePublicRef(publicRef); err != nil {
			return nil, err
		}
		publicRefPtr = &publicRef
	}
	status := surfaceStatus(payload)
	var statusPtr *string
	if status != "" {
		statusPtr = &status
	}
	metadata := surfacePageMetadata(payload, hints)
	return json.Marshal(map[string]any{
		"profile":     surfaceProfile,
		"kind":        "page_record",
		"page_id":     pageID,
		"owner_ura":   ownerURA,
		"surface_ref": surfaceRef,
		"public_ref":  publicRefPtr,
		"status":      statusPtr,
		"metadata":    metadata,
	})
}

func projectSurfacePublicPageRef(raw []byte, hints surfaceProjectionHints) ([]byte, error) {
	recordRaw, err := projectSurfacePageRecord(raw, hints)
	if err != nil {
		return nil, err
	}
	record, err := NewSurfacePageRecordFromJSON(recordRaw)
	if err != nil {
		return nil, err
	}
	if record.PublicRef == nil || strings.TrimSpace(*record.PublicRef) == "" {
		return nil, invalidProfilePayload(surfaceProfile, "public_ref is required", nil)
	}
	return json.Marshal(map[string]any{
		"profile":     surfaceProfile,
		"kind":        "public_page_ref",
		"page_id":     record.PageID,
		"owner_ura":   record.OwnerURA,
		"surface_ref": record.SurfaceRef,
		"public_ref":  *record.PublicRef,
		"route_kind":  "hub_web",
		"metadata": map[string]any{
			"profile":        surfaceProfile,
			"source_ability": surfaceAbilityManifest,
			"raw_page":       json.RawMessage(recordRaw),
		},
	})
}

func projectSurfaceManifest(raw []byte, hints surfaceProjectionHints) ([]byte, error) {
	recordRaw, err := projectSurfacePageRecord(raw, hints)
	if err != nil {
		return nil, err
	}
	record, err := NewSurfacePageRecordFromJSON(recordRaw)
	if err != nil {
		return nil, err
	}
	if record.PublicRef == nil || strings.TrimSpace(*record.PublicRef) == "" {
		return nil, invalidProfilePayload(surfaceProfile, "public_ref is required", nil)
	}
	return json.Marshal(map[string]any{
		"profile":     surfaceProfile,
		"kind":        "surface_manifest",
		"page_id":     record.PageID,
		"owner_ura":   record.OwnerURA,
		"surface_ref": record.SurfaceRef,
		"public_ref":  *record.PublicRef,
		"page":        record,
		"entrypoint": map[string]any{
			"kind": "public_page_ref",
			"href": *record.PublicRef,
		},
		"metadata": map[string]any{
			"profile":        surfaceProfile,
			"source_ability": surfaceAbilityManifest,
			"raw_page":       json.RawMessage(raw),
		},
	})
}

func projectSurfaceMutationResult(raw []byte) ([]byte, error) {
	payload, err := surfaceOutputObject(raw)
	if err != nil {
		return nil, err
	}
	if payload["profile"] == surfaceProfile && payload["kind"] == "surface_mutation_result" {
		return raw, nil
	}
	pageID := firstSurfaceString(payload, "project_id", "page_id", "id")
	if pageID == "" {
		return nil, invalidProfilePayload(surfaceProfile, "surface mutation project_id is required", nil)
	}
	if err := validateSurfaceProjectID(pageID); err != nil {
		return nil, err
	}
	removed, _ := payload["removed"].(bool)
	state := "unknown"
	if removed {
		state = "deleted"
	}
	return json.Marshal(map[string]any{
		"profile":   surfaceProfile,
		"kind":      "surface_mutation_result",
		"operation": firstNonEmpty(firstSurfaceString(payload, "operation"), "delete"),
		"page_id":   pageID,
		"removed":   removed,
		"state":     state,
		"metadata": map[string]any{
			"profile":        surfaceProfile,
			"source_ability": surfaceAbilityDeletePage,
			"raw_result":     json.RawMessage(raw),
		},
	})
}

func projectSurfaceHealth(raw []byte, base SurfaceCarrierBase) ([]byte, error) {
	payload, err := surfaceOutputObject(raw)
	if err != nil {
		return nil, err
	}
	if payload["profile"] == surfaceProfile && payload["kind"] == "surface_health" {
		return raw, nil
	}
	result := payload
	if nested, ok := payload["result"].(map[string]any); ok {
		result = nested
	}
	ownerURA := firstNonEmpty(firstSurfaceString(result, "owner_ura"), firstSurfaceString(payload, "owner_ura"), base.CalleeURA)
	surfaceRef := firstNonEmpty(firstSurfaceString(result, "surface_ref", "project_ura", "resource_ura"), firstSurfaceString(payload, "surface_ref", "project_ura", "resource_ura"))
	if surfaceRef == "" {
		if recordRaw, err := projectSurfacePageRecord(raw, surfaceProjectionHints{OwnerURA: ownerURA, Realm: realmFromURA(ownerURA)}); err == nil {
			record, recordErr := NewSurfacePageRecordFromJSON(recordRaw)
			if recordErr == nil {
				surfaceRef = record.SurfaceRef
			}
		}
	}
	if ownerURA == "" || surfaceRef == "" {
		return nil, invalidProfilePayload(surfaceProfile, "surface health owner_ura and surface_ref are required", nil)
	}
	descriptorVersion := firstNonEmpty(firstSurfaceString(result, "descriptor_version"), firstSurfaceString(payload, "descriptor_version"), base.DescriptorVersion)
	if descriptorVersion == "" {
		descriptorVersion = "1.0.0"
	}
	descriptorRef := firstNonEmpty(firstSurfaceString(result, "descriptor_ref"), firstSurfaceString(payload, "descriptor_ref"))
	if descriptorRef == "" {
		descriptorRef = fmt.Sprintf("%s.%s@%s", strings.Replace(ownerURA, "/agent/", "/ability/", 1), surfaceAbilityHealth, descriptorVersion)
	}
	checks := surfaceHealthChecks(result)
	state := firstSurfaceString(result, "state")
	if state == "" {
		state = "ready"
		for _, check := range checks {
			if ready, _ := check["ready"].(bool); !ready {
				state = "degraded"
				break
			}
		}
	}
	ready, ok := result["ready"].(bool)
	if !ok {
		ready = state == "ready" || state == "healthy" || state == "ok"
		for _, check := range checks {
			if checkReady, _ := check["ready"].(bool); !checkReady {
				ready = false
				break
			}
		}
	}
	return json.Marshal(map[string]any{
		"profile":            surfaceProfile,
		"kind":               "surface_health",
		"state":              state,
		"ready":              ready,
		"owner_ura":          ownerURA,
		"surface_ref":        surfaceRef,
		"descriptor_ref":     descriptorRef,
		"descriptor_version": descriptorVersion,
		"page_count":         surfacePageCount(result),
		"checks":             checks,
		"metadata": map[string]any{
			"profile":         surfaceProfile,
			"source_ability":  surfaceAbilityHealth,
			"rendering_owner": "backend",
			"raw_health":      json.RawMessage(raw),
		},
	})
}

func surfaceOutputObject(raw []byte) (map[string]any, error) {
	var payload map[string]any
	if err := json.Unmarshal(raw, &payload); err != nil {
		return nil, invalidProfilePayload(surfaceProfile, fmt.Sprintf("decode surface output: %v", err), err)
	}
	if payload == nil {
		return nil, invalidProfilePayload(surfaceProfile, "surface output must be an object", nil)
	}
	return payload, nil
}

func surfaceRows(payload map[string]any) ([][]byte, error) {
	for _, key := range []string{"projects", "items"} {
		rawRows, ok := payload[key].([]any)
		if !ok {
			continue
		}
		rows := make([][]byte, 0, len(rawRows))
		for _, row := range rawRows {
			raw, err := json.Marshal(row)
			if err != nil {
				return nil, invalidProfilePayload(surfaceProfile, fmt.Sprintf("encode surface row: %v", err), err)
			}
			rows = append(rows, raw)
		}
		return rows, nil
	}
	if result, ok := payload["result"].(map[string]any); ok {
		return surfaceRows(result)
	}
	return nil, invalidProfilePayload(surfaceProfile, "surface page rows are required", nil)
}

func surfaceCursorOffset(cursor string) (int, error) {
	if strings.TrimSpace(cursor) == "" {
		return 0, nil
	}
	var offset int
	if _, err := fmt.Sscanf(cursor, "%d", &offset); err != nil || offset < 0 {
		return 0, invalidProfilePayload(surfaceProfile, "surface cursor must be a non-negative offset", err)
	}
	return offset, nil
}

func normalizedSurfaceLimit(limit int) int {
	if limit == 0 {
		return DefaultSurfacePageSize
	}
	return limit
}

func firstSurfaceString(values map[string]any, keys ...string) string {
	for _, key := range keys {
		value, _ := values[key].(string)
		if strings.TrimSpace(value) != "" {
			return strings.TrimSpace(value)
		}
	}
	return ""
}

func realmFromURA(value string) string {
	const marker = "easynet:///r/"
	if !strings.HasPrefix(value, marker) {
		return ""
	}
	rest := strings.TrimPrefix(value, marker)
	realm, _, ok := strings.Cut(rest, "/")
	if !ok {
		return ""
	}
	return realm
}

func validateSurfacePublicRef(value string) error {
	if strings.HasPrefix(value, "https://") || strings.HasPrefix(value, "http://127.0.0.1") {
		return nil
	}
	if strings.HasPrefix(value, "http://") && strings.Contains(value, ".pages.localhost") {
		return nil
	}
	return invalidProfilePayload(surfaceProfile, "public_ref must be an https URL or daemon-local pages localhost URL", nil)
}

func surfaceStatus(payload map[string]any) string {
	status := firstSurfaceString(payload, "status")
	if status != "" {
		return status
	}
	visibility := firstSurfaceString(payload, "visibility")
	if visibility == "public" || visibility == "private" {
		return "published"
	}
	return visibility
}

func surfacePageMetadata(payload map[string]any, hints surfaceProjectionHints) map[string]any {
	metadata := map[string]any{}
	if rawMetadata, ok := payload["metadata"].(map[string]any); ok {
		for key, value := range rawMetadata {
			metadata[key] = value
		}
	}
	for _, key := range []string{"user", "project_id", "folder", "visibility", "started_at_ms", "dev_listener_url_root", "file_size_cap", "host_node_id"} {
		if value, ok := payload[key]; ok {
			metadata[key] = value
		}
	}
	if hints.SelectedNodeID != "" {
		metadata["selected_node_id"] = hints.SelectedNodeID
	}
	metadata["profile"] = surfaceProfile
	metadata["source_ability"] = surfaceAbilityManifest
	metadata["raw_page"] = payload
	return metadata
}

func surfaceHealthChecks(payload map[string]any) []map[string]any {
	rawChecks, ok := payload["checks"].([]any)
	if !ok {
		state := firstNonEmpty(firstSurfaceString(payload, "state"), "ready")
		ready, ok := payload["ready"].(bool)
		if !ok {
			ready = state == "ready" || state == "healthy" || state == "ok"
		}
		return []map[string]any{{
			"name":       "surface",
			"state":      state,
			"ready":      ready,
			"message":    surfaceValueOrNull(payload["message"]),
			"latency_ms": int64(0),
			"metadata":   map[string]any{"source": surfaceAbilityHealth},
		}}
	}
	checks := make([]map[string]any, 0, len(rawChecks))
	for _, rawCheck := range rawChecks {
		check, _ := rawCheck.(map[string]any)
		if check == nil {
			continue
		}
		state := firstNonEmpty(firstSurfaceString(check, "state"), "ready")
		ready, ok := check["ready"].(bool)
		if !ok {
			ready = state == "ready" || state == "healthy" || state == "ok"
		}
		metadata, _ := check["metadata"].(map[string]any)
		if metadata == nil {
			metadata = map[string]any{}
		}
		checks = append(checks, map[string]any{
			"name":       firstNonEmpty(firstSurfaceString(check, "name"), "surface"),
			"state":      state,
			"ready":      ready,
			"message":    surfaceValueOrNull(check["message"]),
			"latency_ms": surfaceNumericInt64(check["latency_ms"]),
			"metadata":   metadata,
		})
	}
	return checks
}

func surfacePageCount(payload map[string]any) int {
	if count := surfaceNumericInt64(payload["page_count"]); count > 0 {
		return int(count)
	}
	for _, key := range []string{"projects", "pages", "items"} {
		if rows, ok := payload[key].([]any); ok {
			return len(rows)
		}
	}
	if firstSurfaceString(payload, "page_id", "project_id") != "" {
		return 1
	}
	return 0
}

func surfaceNumericInt64(value any) int64 {
	switch typed := value.(type) {
	case int:
		return int64(typed)
	case int64:
		return typed
	case float64:
		return int64(typed)
	case json.Number:
		n, _ := typed.Int64()
		return n
	default:
		return 0
	}
}

func surfaceValueOrNull(value any) any {
	if value == nil {
		return nil
	}
	return value
}
