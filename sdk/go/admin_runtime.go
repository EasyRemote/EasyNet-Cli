package easynet

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
)

const (
	adminAbilityAgentList        = "agent.list"
	adminAbilityAgentStart       = "agent.start"
	adminAbilityAgentStop        = "agent.stop"
	adminAbilityAgentRefresh     = "agent.refresh"
	adminAbilitySessionList      = "session.list"
	adminAbilitySessionCreate    = "session.create"
	adminAbilitySessionDelete    = "session.delete"
	adminAbilityHubJoin          = "hub.join"
	adminAbilityHubLeave         = "hub.leave"
	adminAbilityPairingPreflight = "pairing.preflight"
	adminAbilityPairingValidate  = "pairing.validate"
	adminAbilityCredentialVerify = "credential.verify"
	adminAbilityPairingCreate    = "pairing.create"
	adminAbilityRevokeDevice     = "federation.revoke"
)

// AdminRuntimeTransport lowers Admin + Gateway requests into Runtime Core
// invocations and projects daemon lifecycle facts into Admin DTOs.
type AdminRuntimeTransport struct {
	runtime        *RuntimeClient
	identity       *IdentityClient
	statusProvider AdminGatewayStatusProvider
}

// AdminGatewayStatusProvider supplies daemon-owned GatewayStatus projections to
// the runtime Admin facade. Ability-backed admin operations still go through
// Runtime Core; lifecycle/status facts must come from an explicit status seam.
type AdminGatewayStatusProvider interface {
	GatewayStatus(ctx context.Context, requestJSON []byte) ([]byte, error)
}

type AdminGatewayStatusProviderFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)

func (f AdminGatewayStatusProviderFunc) GatewayStatus(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f == nil {
		return nil, invalidProfileClient(adminGatewayProfile, "admin gateway-status provider function is required")
	}
	return f(ctx, requestJSON)
}

func NewAdminRuntimeTransport(runtime *RuntimeClient, identity *IdentityClient) (*AdminRuntimeTransport, error) {
	return NewAdminRuntimeTransportWithGatewayStatus(runtime, identity, nil)
}

func NewAdminRuntimeTransportWithGatewayStatus(runtime *RuntimeClient, identity *IdentityClient, statusProvider AdminGatewayStatusProvider) (*AdminRuntimeTransport, error) {
	if runtime == nil {
		return nil, invalidProfileClient(adminGatewayProfile, "runtime client is required")
	}
	if identity == nil {
		return nil, invalidProfileClient(adminGatewayProfile, "identity client is required")
	}
	return &AdminRuntimeTransport{
		runtime:        runtime,
		identity:       identity,
		statusProvider: statusProvider,
	}, nil
}

func NewRuntimeAdminClient(runtime *RuntimeClient, identity *IdentityClient) (*AdminClient, error) {
	return NewRuntimeAdminClientWithGatewayStatus(runtime, identity, nil)
}

func NewRuntimeAdminClientWithGatewayStatus(runtime *RuntimeClient, identity *IdentityClient, statusProvider AdminGatewayStatusProvider) (*AdminClient, error) {
	transport, err := NewAdminRuntimeTransportWithGatewayStatus(runtime, identity, statusProvider)
	if err != nil {
		return nil, err
	}
	return NewAdminClient(transport)
}

func (t *AdminRuntimeTransport) BuildAgentListInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[AdminAgentListRequest](requestJSON, validateAdminAgentListRequest)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, request.AdminCarrierBase, adminAbilityAgentList, map[string]any{})
}

func (t *AdminRuntimeTransport) BuildAgentStartInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[AdminAgentStartRequest](requestJSON, validateAdminAgentStartRequest)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, request.AdminCarrierBase, adminAbilityAgentStart, agentStartArgs(request))
}

func (t *AdminRuntimeTransport) BuildAgentStopInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[AdminAgentStopRequest](requestJSON, validateAdminAgentStopRequest)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, request.AdminCarrierBase, adminAbilityAgentStop, agentStopArgs(request))
}

func (t *AdminRuntimeTransport) BuildAgentRefreshInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[AdminAgentRefreshRequest](requestJSON, validateAdminAgentRefreshRequest)
	if err != nil {
		return nil, err
	}
	args := map[string]any{}
	if strings.TrimSpace(request.Name) != "" {
		args["name"] = strings.TrimSpace(request.Name)
	}
	return t.buildInvocationJSON(ctx, request.AdminCarrierBase, adminAbilityAgentRefresh, args)
}

func (t *AdminRuntimeTransport) BuildSessionListInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[AdminSessionListRequest](requestJSON, validateAdminSessionListRequest)
	if err != nil {
		return nil, err
	}
	args := map[string]any{}
	if request.IncludeTerminated != nil {
		args["include_terminated"] = *request.IncludeTerminated
	}
	return t.buildInvocationJSON(ctx, request.AdminCarrierBase, adminAbilitySessionList, args)
}

func (t *AdminRuntimeTransport) BuildRevokeDeviceInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[RevokeDeviceRequest](requestJSON, validateRevokeDeviceRequest)
	if err != nil {
		return nil, err
	}
	args := map[string]any{
		"agent_ura": request.DeviceURA,
		"reason":    request.Reason,
	}
	return t.buildInvocationJSON(ctx, request.AdminCarrierBase, adminAbilityRevokeDevice, args)
}

func (t *AdminRuntimeTransport) GatewayStatus(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if ctx == nil {
		return nil, invalidProfileClient(adminGatewayProfile, "context is required")
	}
	if t == nil || t.statusProvider == nil {
		return nil, invalidProfileClient(adminGatewayProfile, "admin runtime gateway status provider is required")
	}
	raw, err := t.statusProvider.GatewayStatus(ctx, requestJSON)
	if err != nil {
		return nil, err
	}
	if _, err := NewGatewayStatusFromJSON(raw); err != nil {
		return nil, err
	}
	return raw, nil
}

func (t *AdminRuntimeTransport) ListAgents(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[AdminAgentListRequest](requestJSON, validateAdminAgentListRequest)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, request.AdminCarrierBase, adminAbilityAgentList, map[string]any{})
	if err != nil {
		return nil, err
	}
	return projectAdminAgentPage(output)
}

func (t *AdminRuntimeTransport) AgentStart(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[AdminAgentStartRequest](requestJSON, validateAdminAgentStartRequest)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, request.AdminCarrierBase, adminAbilityAgentStart, agentStartArgs(request))
	if err != nil {
		return nil, err
	}
	return projectAdminLifecycleResult(output, adminAbilityAgentStart, nil)
}

func (t *AdminRuntimeTransport) AgentStop(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[AdminAgentStopRequest](requestJSON, validateAdminAgentStopRequest)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, request.AdminCarrierBase, adminAbilityAgentStop, agentStopArgs(request))
	if err != nil {
		return nil, err
	}
	return projectAdminLifecycleResult(output, adminAbilityAgentStop, nil)
}

func (t *AdminRuntimeTransport) AgentRefresh(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[AdminAgentRefreshRequest](requestJSON, validateAdminAgentRefreshRequest)
	if err != nil {
		return nil, err
	}
	args := map[string]any{}
	if strings.TrimSpace(request.Name) != "" {
		args["name"] = strings.TrimSpace(request.Name)
	}
	output, err := t.invoke(ctx, request.AdminCarrierBase, adminAbilityAgentRefresh, args)
	if err != nil {
		return nil, err
	}
	return projectAdminLifecycleResult(output, adminAbilityAgentRefresh, nil)
}

func (t *AdminRuntimeTransport) ListDeviceSessions(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[AdminSessionListRequest](requestJSON, validateAdminSessionListRequest)
	if err != nil {
		return nil, err
	}
	args := map[string]any{}
	if request.IncludeTerminated != nil {
		args["include_terminated"] = *request.IncludeTerminated
	}
	output, err := t.invoke(ctx, request.AdminCarrierBase, adminAbilitySessionList, args)
	if err != nil {
		return nil, err
	}
	return projectAdminDeviceSessionPage(output)
}

func (t *AdminRuntimeTransport) JoinHub(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[AdminJoinHubRequest](requestJSON, validateAdminJoinHubRequest)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, request.AdminCarrierBase, adminAbilityHubJoin, hubJoinArgs(request))
	if err != nil {
		return nil, err
	}
	return projectAdminLifecycleResult(output, adminAbilityHubJoin, &request.DeviceURA)
}

func (t *AdminRuntimeTransport) LeaveHub(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[AdminLeaveHubRequest](requestJSON, validateAdminLeaveHubRequest)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, request.AdminCarrierBase, adminAbilityHubLeave, hubLeaveArgs(request))
	if err != nil {
		return nil, err
	}
	return projectAdminLifecycleResult(output, adminAbilityHubLeave, nil)
}

func (t *AdminRuntimeTransport) PairingPreflight(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[PairingPreflightRequest](requestJSON, validatePairingPreflightRequest)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, request.AdminCarrierBase, adminAbilityPairingPreflight, pairingPreflightArgs(request))
	if err != nil {
		return nil, err
	}
	return projectPairingPreflight(output, request)
}

func (t *AdminRuntimeTransport) ValidatePairing(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[ValidatePairingRequest](requestJSON, validatePairingValidationRequest)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, request.AdminCarrierBase, adminAbilityPairingValidate, pairingValidateArgs(request))
	if err != nil {
		return nil, err
	}
	return projectDeviceCredential(output, request)
}

func (t *AdminRuntimeTransport) VerifyDeviceCredential(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[VerifyDeviceCredentialRequest](requestJSON, validateDeviceCredentialVerificationRequest)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, request.AdminCarrierBase, adminAbilityCredentialVerify, credentialVerifyArgs(request))
	if err != nil {
		return nil, err
	}
	return projectDeviceCredentialVerification(output, request)
}

func (t *AdminRuntimeTransport) CreatePairing(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[CreatePairingRequest](requestJSON, validateCreatePairingRequest)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, request.AdminCarrierBase, adminAbilityPairingCreate, pairingCreateArgs(request))
	if err != nil {
		return nil, err
	}
	return projectPairingToken(output, request)
}

func (t *AdminRuntimeTransport) RevokeDevice(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[RevokeDeviceRequest](requestJSON, validateRevokeDeviceRequest)
	if err != nil {
		return nil, err
	}
	args := map[string]any{
		"agent_ura": request.DeviceURA,
		"reason":    request.Reason,
	}
	output, err := t.invoke(ctx, request.AdminCarrierBase, adminAbilityRevokeDevice, args)
	if err != nil {
		return nil, err
	}
	return projectAdminLifecycleResult(output, adminAbilityRevokeDevice, &request.DeviceURA)
}

func (t *AdminRuntimeTransport) CreateDeviceSession(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[CreateDeviceSessionRequest](requestJSON, validateCreateDeviceSessionRequest)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, request.AdminCarrierBase, adminAbilitySessionCreate, deviceSessionCreateArgs(request))
	if err != nil {
		return nil, err
	}
	return projectAdminDeviceSession(output, request)
}

func (t *AdminRuntimeTransport) DeleteDeviceSession(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeAdminRuntimeRequest[DeleteDeviceSessionRequest](requestJSON, validateDeleteDeviceSessionRequest)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, request.AdminCarrierBase, adminAbilitySessionDelete, deviceSessionDeleteArgs(request))
	if err != nil {
		return nil, err
	}
	return projectAdminLifecycleResult(output, adminAbilitySessionDelete, nil)
}

func (t *AdminRuntimeTransport) Close(context.Context) error {
	return nil
}

func (t *AdminRuntimeTransport) buildInvocationJSON(ctx context.Context, base AdminCarrierBase, abilityName string, args any) ([]byte, error) {
	draft, err := t.buildInvocation(ctx, base, abilityName, args)
	if err != nil {
		return nil, err
	}
	raw, err := json.Marshal(draft)
	if err != nil {
		return nil, invalidProfilePayload(adminGatewayProfile, fmt.Sprintf("encode admin invocation: %v", err), err)
	}
	return raw, nil
}

func (t *AdminRuntimeTransport) buildInvocation(ctx context.Context, base AdminCarrierBase, abilityName string, args any) (InvocationDraft, error) {
	if t == nil || t.runtime == nil || t.identity == nil {
		return InvocationDraft{}, invalidProfileClient(adminGatewayProfile, "admin runtime transport is not initialized")
	}
	if ctx == nil {
		return InvocationDraft{}, invalidProfileClient(adminGatewayProfile, "context is required")
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
		WithMetadata(adminRuntimeMetadata(base.Metadata, abilityName)).
		Build()
}

func (t *AdminRuntimeTransport) invoke(ctx context.Context, base AdminCarrierBase, abilityName string, args any) ([]byte, error) {
	draft, err := t.buildInvocation(ctx, base, abilityName, args)
	if err != nil {
		return nil, err
	}
	result, err := t.runtime.Invoke(ctx, draft)
	if err != nil {
		return nil, err
	}
	if !result.OK() {
		return nil, adminInvocationFailureError(result)
	}
	outputJSON := result.OutputJSON()
	if len(outputJSON) == 0 || string(outputJSON) == "null" {
		return []byte(`{}`), nil
	}
	return outputJSON, nil
}

func decodeAdminRuntimeRequest[T any](requestJSON []byte, validate func(any) error) (T, error) {
	var request T
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		return request, invalidProfilePayload(adminGatewayProfile, fmt.Sprintf("decode admin request: %v", err), err)
	}
	if validate != nil {
		if err := validate(any(request)); err != nil {
			return request, err
		}
	}
	return request, nil
}

func agentStartArgs(request AdminAgentStartRequest) map[string]any {
	args := map[string]any{
		"name": request.Name,
	}
	if request.AgentType != "" {
		args["agent_type"] = request.AgentType
	}
	if len(request.Entry) > 0 {
		args["entry"] = request.Entry
	}
	for key, value := range map[string]string{
		"model":     request.Model,
		"label":     request.Label,
		"command":   request.Command,
		"root_path": request.RootPath,
	} {
		if strings.TrimSpace(value) != "" {
			args[key] = value
		}
	}
	if len(request.CommandArgs) > 0 {
		args["command_args"] = request.CommandArgs
	}
	if request.ModelPresent != nil {
		args["model_present"] = *request.ModelPresent
	}
	if request.MaterializeDirectory != nil {
		args["materialize_directory"] = *request.MaterializeDirectory
	}
	if request.UpdateExistingSpec != nil {
		args["update_existing_spec"] = *request.UpdateExistingSpec
	}
	if request.ProjectWorkspace != nil {
		args["project_workspace"] = *request.ProjectWorkspace
	}
	return args
}

func agentStopArgs(request AdminAgentStopRequest) map[string]any {
	args := map[string]any{}
	if strings.TrimSpace(request.Name) != "" {
		args["name"] = strings.TrimSpace(request.Name)
	}
	if strings.TrimSpace(request.AgentURA) != "" {
		args["agent_ura"] = strings.TrimSpace(request.AgentURA)
	}
	return args
}

func deviceSessionCreateArgs(request CreateDeviceSessionRequest) map[string]any {
	args := map[string]any{
		"device_ura":   strings.TrimSpace(request.DeviceURA),
		"hub_ura":      strings.TrimSpace(request.HubURA),
		"session_kind": strings.TrimSpace(request.SessionKind),
	}
	if request.ExpiresUnixMS > 0 {
		args["expires_unix_ms"] = request.ExpiresUnixMS
	}
	return args
}

func deviceSessionDeleteArgs(request DeleteDeviceSessionRequest) map[string]any {
	args := map[string]any{
		"session_id": strings.TrimSpace(request.SessionID),
	}
	if strings.TrimSpace(request.Reason) != "" {
		args["reason"] = strings.TrimSpace(request.Reason)
	}
	return args
}

func hubJoinArgs(request AdminJoinHubRequest) map[string]any {
	return map[string]any{
		"hub_ura":    strings.TrimSpace(request.HubURA),
		"device_ura": strings.TrimSpace(request.DeviceURA),
	}
}

func hubLeaveArgs(request AdminLeaveHubRequest) map[string]any {
	args := map[string]any{
		"hub_ura": strings.TrimSpace(request.HubURA),
	}
	if strings.TrimSpace(request.Reason) != "" {
		args["reason"] = strings.TrimSpace(request.Reason)
	}
	return args
}

func pairingPreflightArgs(request PairingPreflightRequest) map[string]any {
	args := map[string]any{
		"hub_ura":    strings.TrimSpace(request.HubURA),
		"device_ura": strings.TrimSpace(request.DeviceURA),
	}
	if len(request.RequestedScopes) > 0 {
		args["requested_scopes"] = request.RequestedScopes
	}
	return args
}

func pairingValidateArgs(request ValidatePairingRequest) map[string]any {
	return map[string]any{
		"token":      strings.TrimSpace(request.Token),
		"device_ura": strings.TrimSpace(request.DeviceURA),
	}
}

func credentialVerifyArgs(request VerifyDeviceCredentialRequest) map[string]any {
	return map[string]any{
		"credential_id": strings.TrimSpace(request.CredentialID),
		"device_ura":    strings.TrimSpace(request.DeviceURA),
		"hub_ura":       strings.TrimSpace(request.HubURA),
	}
}

func pairingCreateArgs(request CreatePairingRequest) map[string]any {
	args := map[string]any{
		"hub_ura":         strings.TrimSpace(request.HubURA),
		"device_ura":      strings.TrimSpace(request.DeviceURA),
		"expires_unix_ms": request.ExpiresUnixMS,
	}
	if len(request.Scopes) > 0 {
		args["scopes"] = request.Scopes
	}
	return args
}

func adminRuntimeMetadata(input map[string]any, abilityName string) map[string]any {
	metadata := map[string]any{}
	for key, value := range input {
		metadata[key] = value
	}
	metadata["profile"] = adminGatewayProfile
	metadata["system_ability"] = abilityName
	metadata["carrier_owner"] = "daemon_sdk"
	return metadata
}

func projectAdminAgentPage(raw []byte) ([]byte, error) {
	payload, err := adminOutputObject(raw)
	if err != nil {
		return nil, err
	}
	rows := firstArray(payload, "agents", "items")
	items := make([]any, 0, len(rows))
	for _, row := range rows {
		obj, ok := row.(map[string]any)
		if !ok {
			return nil, invalidProfilePayload(adminGatewayProfile, "agent row must be an object", nil)
		}
		items = append(items, map[string]any{
			"name":       firstStringFromMap(obj, "name", "id"),
			"agent_ura":  firstStringPtr(firstStringFromMap(obj, "agent_ura", "agentUra", "ura")),
			"owner_ura":  firstStringPtr(firstStringFromMap(obj, "owner_ura", "ownerUra")),
			"device_ura": firstStringPtr(firstStringFromMap(obj, "device_ura", "deviceUra")),
			"state":      firstNonEmpty(firstStringFromMap(obj, "state", "status"), "unknown"),
			"runtime":    firstNonEmpty(firstStringFromMap(obj, "runtime"), "daemon"),
			"model":      firstStringPtr(firstStringFromMap(obj, "model")),
			"label":      firstStringPtr(firstStringFromMap(obj, "label")),
			"abilities":  firstNonNilValue(obj, []any{}, "abilities"),
			"metadata":   adminMetadataWithSource(obj, adminAbilityAgentList),
		})
	}
	return json.Marshal(map[string]any{
		"profile":     adminGatewayProfile,
		"kind":        "agent_records",
		"state":       "ok",
		"items":       items,
		"next_cursor": nil,
		"metadata": map[string]any{
			"profile": adminGatewayProfile,
			"source":  adminAbilityAgentList,
			"count":   len(items),
		},
	})
}

func projectAdminDeviceSessionPage(raw []byte) ([]byte, error) {
	payload, err := adminOutputObject(raw)
	if err != nil {
		return nil, err
	}
	rows := firstArray(payload, "sessions", "items")
	return json.Marshal(map[string]any{
		"profile":     adminGatewayProfile,
		"kind":        "device_sessions",
		"state":       firstNonEmpty(firstStringFromMap(payload, "state"), "ok"),
		"items":       rows,
		"next_cursor": firstValue(payload, "next_cursor", "nextCursor"),
		"metadata": map[string]any{
			"profile": adminGatewayProfile,
			"source":  adminAbilitySessionList,
			"count":   len(rows),
		},
	})
}

func projectAdminDeviceSession(raw []byte, request CreateDeviceSessionRequest) ([]byte, error) {
	payload, err := adminOutputObject(raw)
	if err != nil {
		return nil, err
	}
	if payload["profile"] == adminGatewayProfile && payload["kind"] == "device_session" {
		projected, err := json.Marshal(payload)
		if err != nil {
			return nil, invalidProfilePayload(adminGatewayProfile, fmt.Sprintf("encode daemon device session projection: %v", err), err)
		}
		if _, err := NewDeviceSessionFromJSON(projected); err != nil {
			return nil, err
		}
		return projected, nil
	}
	session := map[string]any{
		"profile":         adminGatewayProfile,
		"kind":            "device_session",
		"session_id":      firstNonEmpty(firstStringFromMap(payload, "session_id", "sessionId", "id"), firstStringFromMap(payload, "session")),
		"device_ura":      firstNonEmpty(firstStringFromMap(payload, "device_ura", "deviceUra"), strings.TrimSpace(request.DeviceURA)),
		"hub_ura":         firstNonEmpty(firstStringFromMap(payload, "hub_ura", "hubUra"), strings.TrimSpace(request.HubURA)),
		"state":           firstNonEmpty(firstStringFromMap(payload, "state", "status"), "active"),
		"session_kind":    firstNonEmpty(firstStringFromMap(payload, "session_kind", "sessionKind", "kind"), strings.TrimSpace(request.SessionKind)),
		"created_unix_ms": firstAdminNumericInt64(payload, "created_unix_ms", "createdUnixMs", "created_at_ms"),
		"expires_unix_ms": firstNonZeroAdminInt64(firstAdminNumericInt64(payload, "expires_unix_ms", "expiresUnixMs", "expires_at_ms"), request.ExpiresUnixMS),
		"metadata": map[string]any{
			"profile":    adminGatewayProfile,
			"source":     adminAbilitySessionCreate,
			"raw_result": payload,
		},
	}
	rawSession, err := json.Marshal(session)
	if err != nil {
		return nil, invalidProfilePayload(adminGatewayProfile, fmt.Sprintf("encode device session projection: %v", err), err)
	}
	if _, err := NewDeviceSessionFromJSON(rawSession); err != nil {
		return nil, err
	}
	return rawSession, nil
}

func projectPairingPreflight(raw []byte, request PairingPreflightRequest) ([]byte, error) {
	payload, err := adminOutputObject(raw)
	if err != nil {
		return nil, err
	}
	if payload["profile"] == adminGatewayProfile && payload["kind"] == "pairing_preflight" {
		return validatedPairingPreflightJSON(payload)
	}
	preflight := map[string]any{
		"profile":          adminGatewayProfile,
		"kind":             "pairing_preflight",
		"state":            firstNonEmpty(firstStringFromMap(payload, "state", "status"), "unknown"),
		"hub_ura":          firstNonEmpty(firstStringFromMap(payload, "hub_ura", "hubUra"), strings.TrimSpace(request.HubURA)),
		"device_ura":       firstNonEmpty(firstStringFromMap(payload, "device_ura", "deviceUra"), strings.TrimSpace(request.DeviceURA)),
		"pairing_required": boolArg(payload, "pairing_required"),
		"trust_ready":      boolArg(payload, "trust_ready"),
		"scopes":           adminRequiredStringArray(payload, "scopes", "requested_scopes", "granted_scopes"),
		"metadata": map[string]any{
			"profile":    adminGatewayProfile,
			"source":     adminAbilityPairingPreflight,
			"raw_result": payload,
		},
	}
	return validatedPairingPreflightJSON(preflight)
}

func projectPairingToken(raw []byte, request CreatePairingRequest) ([]byte, error) {
	payload, err := adminOutputObject(raw)
	if err != nil {
		return nil, err
	}
	if payload["profile"] == adminGatewayProfile && payload["kind"] == "pairing_token" {
		return validatedPairingTokenJSON(payload)
	}
	token := map[string]any{
		"profile":         adminGatewayProfile,
		"kind":            "pairing_token",
		"token_id":        firstStringFromMap(payload, "token_id", "tokenId", "id"),
		"token":           firstStringFromMap(payload, "token", "pairing_token", "pairingToken"),
		"hub_ura":         firstNonEmpty(firstStringFromMap(payload, "hub_ura", "hubUra"), strings.TrimSpace(request.HubURA)),
		"device_ura":      firstNonEmpty(firstStringFromMap(payload, "device_ura", "deviceUra"), strings.TrimSpace(request.DeviceURA)),
		"state":           firstNonEmpty(firstStringFromMap(payload, "state", "status"), "issued"),
		"expires_unix_ms": firstAdminNumericInt64(payload, "expires_unix_ms", "expiresUnixMs", "expires_at_ms"),
		"scopes":          adminRequiredStringArray(payload, "scopes", "granted_scopes"),
		"metadata": map[string]any{
			"profile":    adminGatewayProfile,
			"source":     adminAbilityPairingCreate,
			"raw_result": payload,
		},
	}
	return validatedPairingTokenJSON(token)
}

func projectDeviceCredential(raw []byte, request ValidatePairingRequest) ([]byte, error) {
	payload, err := adminOutputObject(raw)
	if err != nil {
		return nil, err
	}
	if payload["profile"] == adminGatewayProfile && payload["kind"] == "device_credential" {
		return validatedDeviceCredentialJSON(payload)
	}
	credential := map[string]any{
		"profile":         adminGatewayProfile,
		"kind":            "device_credential",
		"credential_id":   firstStringFromMap(payload, "credential_id", "credentialId", "id"),
		"device_ura":      firstNonEmpty(firstStringFromMap(payload, "device_ura", "deviceUra"), strings.TrimSpace(request.DeviceURA)),
		"hub_ura":         firstStringFromMap(payload, "hub_ura", "hubUra"),
		"state":           firstNonEmpty(firstStringFromMap(payload, "state", "status"), "active"),
		"issued_unix_ms":  firstAdminNumericInt64(payload, "issued_unix_ms", "issuedUnixMs", "created_unix_ms"),
		"expires_unix_ms": firstAdminNumericInt64(payload, "expires_unix_ms", "expiresUnixMs", "expires_at_ms"),
		"scopes":          adminRequiredStringArray(payload, "scopes", "granted_scopes"),
		"metadata": map[string]any{
			"profile":    adminGatewayProfile,
			"source":     adminAbilityPairingValidate,
			"raw_result": payload,
		},
	}
	return validatedDeviceCredentialJSON(credential)
}

func projectDeviceCredentialVerification(raw []byte, request VerifyDeviceCredentialRequest) ([]byte, error) {
	payload, err := adminOutputObject(raw)
	if err != nil {
		return nil, err
	}
	if payload["profile"] == adminGatewayProfile && payload["kind"] == "device_credential_verification" {
		return validatedDeviceCredentialVerificationJSON(payload)
	}
	verification := map[string]any{
		"profile":       adminGatewayProfile,
		"kind":          "device_credential_verification",
		"verified":      boolArg(payload, "verified"),
		"credential_id": firstNonEmpty(firstStringFromMap(payload, "credential_id", "credentialId", "id"), strings.TrimSpace(request.CredentialID)),
		"device_ura":    firstNonEmpty(firstStringFromMap(payload, "device_ura", "deviceUra"), strings.TrimSpace(request.DeviceURA)),
		"hub_ura":       firstNonEmpty(firstStringFromMap(payload, "hub_ura", "hubUra"), strings.TrimSpace(request.HubURA)),
		"method":        firstStringFromMap(payload, "method", "verification_method", "verificationMethod"),
		"metadata": map[string]any{
			"profile":    adminGatewayProfile,
			"source":     adminAbilityCredentialVerify,
			"raw_result": payload,
		},
	}
	return validatedDeviceCredentialVerificationJSON(verification)
}

func projectAdminLifecycleResult(raw []byte, operation string, deviceURA *string) ([]byte, error) {
	payload, err := adminOutputObject(raw)
	if err != nil {
		return nil, err
	}
	ack := true
	if value, ok := payload["ack"].(bool); ok {
		ack = value
	}
	state := "ok"
	if !ack {
		state = "not_found"
	}
	if boolArg(payload, "runtime_not_ready") || boolArg(payload, "runtime_catalog_not_ready") {
		state = "not_ready"
	}
	agentURA := firstStringPtr(firstStringFromMap(payload, "agent_ura", "agentUra"))
	resolvedDeviceURA := deviceURA
	if resolvedDeviceURA == nil {
		resolvedDeviceURA = firstStringPtr(firstStringFromMap(payload, "device_ura", "deviceUra"))
	}
	result := map[string]any{
		"profile":                   adminGatewayProfile,
		"kind":                      "admin_result",
		"operation":                 operation,
		"state":                     state,
		"agent_ura":                 agentURA,
		"device_ura":                resolvedDeviceURA,
		"ack":                       ack,
		"runtime_not_ready":         boolArg(payload, "runtime_not_ready"),
		"runtime_catalog_not_ready": boolArg(payload, "runtime_catalog_not_ready"),
		"metadata": map[string]any{
			"profile":    adminGatewayProfile,
			"source":     operation,
			"raw_result": payload,
		},
	}
	return json.Marshal(result)
}

func adminOutputObject(raw []byte) (map[string]any, error) {
	var payload map[string]any
	if err := json.Unmarshal(raw, &payload); err != nil {
		return nil, invalidProfilePayload(adminGatewayProfile, fmt.Sprintf("decode admin output: %v", err), err)
	}
	if payload == nil {
		return nil, invalidProfilePayload(adminGatewayProfile, "admin output must be an object", nil)
	}
	if nested, ok := payload["result"].(map[string]any); ok {
		return nested, nil
	}
	return payload, nil
}

func validatedPairingPreflightJSON(value map[string]any) ([]byte, error) {
	raw, err := json.Marshal(value)
	if err != nil {
		return nil, invalidProfilePayload(adminGatewayProfile, fmt.Sprintf("encode pairing preflight projection: %v", err), err)
	}
	if _, err := NewPairingPreflightFromJSON(raw); err != nil {
		return nil, err
	}
	return raw, nil
}

func validatedPairingTokenJSON(value map[string]any) ([]byte, error) {
	raw, err := json.Marshal(value)
	if err != nil {
		return nil, invalidProfilePayload(adminGatewayProfile, fmt.Sprintf("encode pairing token projection: %v", err), err)
	}
	if _, err := NewPairingTokenFromJSON(raw); err != nil {
		return nil, err
	}
	return raw, nil
}

func validatedDeviceCredentialJSON(value map[string]any) ([]byte, error) {
	raw, err := json.Marshal(value)
	if err != nil {
		return nil, invalidProfilePayload(adminGatewayProfile, fmt.Sprintf("encode device credential projection: %v", err), err)
	}
	if _, err := NewDeviceCredentialFromJSON(raw); err != nil {
		return nil, err
	}
	return raw, nil
}

func validatedDeviceCredentialVerificationJSON(value map[string]any) ([]byte, error) {
	raw, err := json.Marshal(value)
	if err != nil {
		return nil, invalidProfilePayload(adminGatewayProfile, fmt.Sprintf("encode device credential verification projection: %v", err), err)
	}
	if _, err := NewDeviceCredentialVerificationFromJSON(raw); err != nil {
		return nil, err
	}
	return raw, nil
}

func firstAdminNumericInt64(values map[string]any, keys ...string) int64 {
	for _, key := range keys {
		if value, ok := values[key]; ok {
			if numeric, ok := adminNumericInt64(value); ok {
				return numeric
			}
		}
	}
	return 0
}

func adminNumericInt64(value any) (int64, bool) {
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

func firstNonZeroAdminInt64(values ...int64) int64 {
	for _, value := range values {
		if value != 0 {
			return value
		}
	}
	return 0
}

func adminRequiredStringArray(values map[string]any, keys ...string) []string {
	for _, key := range keys {
		raw, ok := values[key]
		if !ok || raw == nil {
			continue
		}
		items, ok := raw.([]any)
		if !ok {
			return nil
		}
		out := make([]string, 0, len(items))
		for _, item := range items {
			text, ok := item.(string)
			if !ok || strings.TrimSpace(text) == "" {
				return nil
			}
			out = append(out, strings.TrimSpace(text))
		}
		return out
	}
	return nil
}

func adminMetadataWithSource(row map[string]any, source string) map[string]any {
	metadata, _ := row["metadata"].(map[string]any)
	out := map[string]any{}
	for key, value := range metadata {
		out[key] = value
	}
	out["profile"] = adminGatewayProfile
	out["source"] = source
	return out
}

func adminInvocationFailureError(result InvocationResult) error {
	failure := result.Failure()
	message := "admin invocation failed"
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
	}, adminGatewayProfile)
}
