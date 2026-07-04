package easynet

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"strings"
)

const (
	identityAbilityRegisterPubkey  = "identity.register_pubkey"
	identityAbilityListUserPubkeys = "identity.list_user_pubkeys"
	identityAbilityRevokePubkey    = "identity.revoke_user_pubkey"
	identityDefaultRole            = "user"
)

// IdentityRuntimeTransport lowers Identity profile requests into Runtime Core
// invocations and projects daemon trust-anchor facts back into SDK DTOs.
type IdentityRuntimeTransport struct {
	runtime  *RuntimeClient
	resolver *IdentityClient
}

func NewIdentityRuntimeTransport(runtime *RuntimeClient, resolver *IdentityClient) (*IdentityRuntimeTransport, error) {
	if runtime == nil {
		return nil, invalidProfileClient(directoryIdentityProfile, "runtime client is required")
	}
	if resolver == nil {
		return nil, invalidProfileClient(directoryIdentityProfile, "identity resolver client is required")
	}
	return &IdentityRuntimeTransport{runtime: runtime, resolver: resolver}, nil
}

func NewRuntimeIdentityClient(runtime *RuntimeClient, resolver *IdentityClient) (*IdentityClient, error) {
	transport, err := NewIdentityRuntimeTransport(runtime, resolver)
	if err != nil {
		return nil, err
	}
	return NewIdentityClient(transport)
}

func (t *IdentityRuntimeTransport) ProjectDescriptorRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	var req DescriptorRefRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode descriptor-ref projection request: %v", err), err)
	}
	projection, err := t.resolver.ProjectDescriptorRef(ctx, req)
	if err != nil {
		return nil, err
	}
	return json.Marshal(projection)
}

func (t *IdentityRuntimeTransport) BuildDescriptorRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	var req DescriptorRefBuildRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode descriptor-ref build request: %v", err), err)
	}
	projection, err := t.resolver.BuildDescriptorRef(ctx, req)
	if err != nil {
		return nil, err
	}
	return json.Marshal(projection)
}

func (t *IdentityRuntimeTransport) ProjectIdentity(ctx context.Context, requestJSON []byte) ([]byte, error) {
	var req IdentityProjectionRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode identity projection request: %v", err), err)
	}
	projection, err := t.resolver.ProjectIdentity(ctx, req)
	if err != nil {
		return nil, err
	}
	return json.Marshal(projection)
}

func (t *IdentityRuntimeTransport) BuildURA(ctx context.Context, requestJSON []byte) ([]byte, error) {
	var req URABuildRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode URA build request: %v", err), err)
	}
	projection, err := t.resolver.BuildURA(ctx, req)
	if err != nil {
		return nil, err
	}
	return json.Marshal(projection)
}

func (t *IdentityRuntimeTransport) BuildResourceRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	var req LocalResourceRefRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode resource-ref request: %v", err), err)
	}
	ref, err := t.resolver.BuildResourceRef(ctx, req)
	if err != nil {
		return nil, err
	}
	return json.Marshal(ref)
}

func (t *IdentityRuntimeTransport) RegisterSigningKey(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeSigningKeyRegistrationForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, requestJSON, request.IdentityCarrierBase, identityAbilityRegisterPubkey)
	if err != nil {
		return nil, err
	}
	return projectIdentitySigningKeyRecord(output, request)
}

func (t *IdentityRuntimeTransport) ListSigningKeys(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeSigningKeyListForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, requestJSON, request.IdentityCarrierBase, identityAbilityListUserPubkeys)
	if err != nil {
		return nil, err
	}
	return projectIdentitySigningKeyPage(output, request)
}

func (t *IdentityRuntimeTransport) RevokeSigningKey(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeSigningKeyRevokeForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, requestJSON, request.IdentityCarrierBase, identityAbilityRevokePubkey)
	if err != nil {
		return nil, err
	}
	return projectIdentitySigningKeyRevokeResult(output, request)
}

func (t *IdentityRuntimeTransport) Signer(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeSignerForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, requestJSON, request.IdentityCarrierBase, identityAbilityListUserPubkeys)
	if err != nil {
		return nil, err
	}
	return projectIdentitySignerHandle(output, request)
}

func (t *IdentityRuntimeTransport) Close(context.Context) error {
	return nil
}

func (t *IdentityRuntimeTransport) buildInvocation(ctx context.Context, requestJSON []byte, base IdentityCarrierBase, abilityName string) (InvocationDraft, error) {
	if t == nil || t.runtime == nil || t.resolver == nil {
		return InvocationDraft{}, invalidProfileClient(directoryIdentityProfile, "identity runtime transport is not initialized")
	}
	if ctx == nil {
		return InvocationDraft{}, invalidProfileClient(directoryIdentityProfile, "context is required")
	}
	payload, err := identityRuntimePayload(requestJSON)
	if err != nil {
		return InvocationDraft{}, err
	}
	descriptorRef, err := t.resolver.OwnerAbilityDescriptorRef(ctx, base.CalleeURA, abilityName, base.DescriptorVersion)
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
		WithJSONArgs(identityRuntimeArgs(payload, abilityName)).
		WithContentType("application/json").
		WithMetadata(identityRuntimeMetadata(base.Metadata, abilityName)).
		Build()
}

func (t *IdentityRuntimeTransport) invoke(ctx context.Context, requestJSON []byte, base IdentityCarrierBase, abilityName string) ([]byte, error) {
	draft, err := t.buildInvocation(ctx, requestJSON, base, abilityName)
	if err != nil {
		return nil, err
	}
	result, err := t.runtime.Invoke(ctx, draft)
	if err != nil {
		return nil, err
	}
	if !result.OK() {
		return nil, identityInvocationFailureError(result)
	}
	outputJSON := result.OutputJSON()
	if len(outputJSON) == 0 || string(outputJSON) == "null" {
		return nil, invalidProfilePayload(directoryIdentityProfile, "identity invocation output_json is required", nil)
	}
	return outputJSON, nil
}

func decodeSigningKeyRegistrationForRuntime(requestJSON []byte) (SigningKeyRegistrationRequest, error) {
	var req SigningKeyRegistrationRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return SigningKeyRegistrationRequest{}, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode signing-key registration request: %v", err), err)
	}
	if _, err := marshalSigningKeyRegistrationRequest(req); err != nil {
		return SigningKeyRegistrationRequest{}, err
	}
	if err := validateIdentityRole(req.Role); err != nil {
		return SigningKeyRegistrationRequest{}, err
	}
	return req, nil
}

func decodeSigningKeyListForRuntime(requestJSON []byte) (SigningKeyListRequest, error) {
	var req SigningKeyListRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return SigningKeyListRequest{}, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode signing-key list request: %v", err), err)
	}
	if _, err := marshalSigningKeyListRequest(req); err != nil {
		return SigningKeyListRequest{}, err
	}
	return req, nil
}

func decodeSigningKeyRevokeForRuntime(requestJSON []byte) (SigningKeyRevokeRequest, error) {
	var req SigningKeyRevokeRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return SigningKeyRevokeRequest{}, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode signing-key revoke request: %v", err), err)
	}
	if _, err := marshalSigningKeyRevokeRequest(req); err != nil {
		return SigningKeyRevokeRequest{}, err
	}
	if err := validateEd25519PublicKey(req.PublicKeyBase64); err != nil {
		return SigningKeyRevokeRequest{}, err
	}
	return req, nil
}

func decodeSignerForRuntime(requestJSON []byte) (SignerRequest, error) {
	var req SignerRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return SignerRequest{}, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode signer request: %v", err), err)
	}
	if _, err := marshalSignerRequest(req); err != nil {
		return SignerRequest{}, err
	}
	return req, nil
}

func identityRuntimePayload(requestJSON []byte) (map[string]any, error) {
	var payload map[string]any
	if err := json.Unmarshal(requestJSON, &payload); err != nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode identity request: %v", err), err)
	}
	if payload == nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, "identity request must be an object", nil)
	}
	return payload, nil
}

func identityRuntimeArgs(payload map[string]any, abilityName string) map[string]any {
	switch abilityName {
	case identityAbilityRegisterPubkey:
		role := firstIdentityString(payload, "role")
		if role == "" {
			role = identityDefaultRole
		}
		return map[string]any{
			"agent_ura":      payload["owner_ura"],
			"public_key_b64": payload["public_key_base64"],
			"role":           role,
		}
	case identityAbilityListUserPubkeys:
		return map[string]any{
			"agent_ura": payload["owner_ura"],
		}
	case identityAbilityRevokePubkey:
		return map[string]any{
			"agent_ura":      payload["owner_ura"],
			"public_key_b64": payload["public_key_base64"],
		}
	default:
		return map[string]any{}
	}
}

func identityRuntimeMetadata(input map[string]any, abilityName string) map[string]any {
	metadata := map[string]any{}
	for key, value := range input {
		metadata[key] = value
	}
	metadata["profile"] = directoryIdentityProfile
	metadata["system_ability"] = abilityName
	metadata["carrier_owner"] = "daemon_sdk"
	return metadata
}

func identityInvocationFailureError(result InvocationResult) error {
	failure := result.Failure()
	message := "identity invocation failed"
	code := ErrAbilityFailed
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
	}, directoryIdentityProfile)
}

func projectIdentitySigningKeyRecord(raw []byte, request SigningKeyRegistrationRequest) ([]byte, error) {
	payload, err := identityOutputObject(raw, "signing-key registration output")
	if err != nil {
		return nil, err
	}
	if !identityOutputOK(payload) {
		return nil, invalidProfilePayload(directoryIdentityProfile, "identity.register_pubkey did not acknowledge success", nil)
	}
	role := request.Role
	if role == "" {
		role = identityDefaultRole
	}
	record := map[string]any{
		"profile":           directoryIdentityProfile,
		"key_id":            request.KeyID,
		"owner_ura":         request.OwnerURA,
		"algorithm":         strings.ToLower(request.Algorithm),
		"public_key_base64": request.PublicKeyBase64,
		"state":             "active",
		"usage":             request.Usage,
		"created_unix_ms":   int64(0),
		"revoked_unix_ms":   int64(0),
		"metadata": map[string]any{
			"source":     identityAbilityRegisterPubkey,
			"daemon_ack": payload,
			"role":       role,
		},
	}
	return json.Marshal(record)
}

func projectIdentitySigningKeyPage(raw []byte, request SigningKeyListRequest) ([]byte, error) {
	payload, err := identityOutputObject(raw, "signing-key list output")
	if err != nil {
		return nil, err
	}
	ownerURA := firstIdentityString(payload, "agent_ura", request.OwnerURA)
	keys, _ := payload["keys"].([]any)
	limit := request.Limit
	if limit == 0 {
		limit = DefaultSigningKeyPageSize
	}
	if limit > len(keys) {
		limit = len(keys)
	}
	items := make([]map[string]any, 0, limit)
	for _, rawKey := range keys[:limit] {
		key, ok := rawKey.(map[string]any)
		if !ok {
			return nil, invalidProfilePayload(directoryIdentityProfile, "identity.list_user_pubkeys keys must be objects", nil)
		}
		publicKey := firstIdentityString(key, "public_key_b64")
		if err := validateEd25519PublicKey(publicKey); err != nil {
			return nil, err
		}
		keyID, err := identityPublicKeyID(publicKey)
		if err != nil {
			return nil, err
		}
		items = append(items, map[string]any{
			"profile":           directoryIdentityProfile,
			"key_id":            keyID,
			"owner_ura":         ownerURA,
			"algorithm":         "ed25519",
			"public_key_base64": publicKey,
			"state":             "active",
			"usage":             []string{"invocation.sign"},
			"created_unix_ms":   identityOptionalInt64(key["added_at_unix_ms"]),
			"revoked_unix_ms":   int64(0),
			"metadata": map[string]any{
				"source":         identityAbilityListUserPubkeys,
				"rotation_epoch": identityOptionalInt64(payload["rotation_epoch"]),
			},
		})
	}
	var nextCursor *string
	if len(keys) > limit {
		cursor := fmt.Sprintf("%d", limit)
		nextCursor = &cursor
	}
	page := map[string]any{
		"profile":     directoryIdentityProfile,
		"items":       items,
		"next_cursor": nextCursor,
		"limit":       firstNonZeroIdentityInt(request.Limit, DefaultSigningKeyPageSize),
		"metadata": map[string]any{
			"source":            identityAbilityListUserPubkeys,
			"owner_ura":         ownerURA,
			"total_available":   len(keys),
			"rotation_epoch":    identityOptionalInt64(payload["rotation_epoch"]),
			"revoked_key_count": identityOptionalInt64(payload["revoked_key_count"]),
		},
	}
	return json.Marshal(page)
}

func projectIdentitySigningKeyRevokeResult(raw []byte, request SigningKeyRevokeRequest) ([]byte, error) {
	payload, err := identityOutputObject(raw, "signing-key revoke output")
	if err != nil {
		return nil, err
	}
	if !identityOutputOK(payload) {
		return nil, invalidProfilePayload(directoryIdentityProfile, "identity.revoke_user_pubkey did not acknowledge success", nil)
	}
	removed := identityOptionalBool(payload["removed"], true)
	result := map[string]any{
		"profile": directoryIdentityProfile,
		"key_id":  request.KeyID,
		"revoked": true,
		"state":   map[bool]string{true: "revoked", false: "not_found"}[removed],
		"metadata": map[string]any{
			"source":  identityAbilityRevokePubkey,
			"removed": removed,
			"reason":  request.Reason,
		},
	}
	return json.Marshal(result)
}

func projectIdentitySignerHandle(raw []byte, request SignerRequest) ([]byte, error) {
	pageJSON, err := projectIdentitySigningKeyPage(raw, SigningKeyListRequest{
		IdentityCarrierBase: request.IdentityCarrierBase,
		OwnerURA:            request.OwnerURA,
		Limit:               MaxSigningKeyPageSize,
	})
	if err != nil {
		return nil, err
	}
	page, err := NewSigningKeyPageFromJSON(pageJSON)
	if err != nil {
		return nil, err
	}
	usage := request.Usage
	if usage == "" {
		usage = "invocation.sign"
	}
	for _, record := range page.Items {
		if record.KeyID != request.KeyID {
			continue
		}
		signerID := "signer-" + record.KeyID
		return json.Marshal(map[string]any{
			"profile":   directoryIdentityProfile,
			"signer_id": signerID,
			"owner_ura": request.OwnerURA,
			"key_id":    record.KeyID,
			"algorithm": "ed25519",
			"policy": map[string]any{
				"mode":      "local_daemon_signing",
				"usage":     usage,
				"signer_id": signerID,
			},
			"metadata": map[string]any{
				"source":            identityAbilityListUserPubkeys,
				"public_key_base64": record.PublicKeyBase64,
				"rotation_epoch":    record.Metadata["rotation_epoch"],
			},
		})
	}
	return nil, invalidProfilePayload(directoryIdentityProfile, "signer key was not present in daemon identity inventory", nil)
}

func identityOutputObject(raw []byte, label string) (map[string]any, error) {
	var payload map[string]any
	if err := json.Unmarshal(raw, &payload); err != nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode %s: %v", label, err), err)
	}
	if payload == nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, label+" must be an object", nil)
	}
	return payload, nil
}

func identityOutputOK(payload map[string]any) bool {
	ok, _ := payload["ok"].(bool)
	return ok
}

func identityPublicKeyID(publicKeyBase64 string) (string, error) {
	decoded, err := decodeEd25519PublicKey(publicKeyBase64)
	if err != nil {
		return "", err
	}
	digest := sha256.Sum256(decoded)
	return "ed25519:" + hex.EncodeToString(digest[:16]), nil
}

func validateEd25519PublicKey(publicKeyBase64 string) error {
	_, err := decodeEd25519PublicKey(publicKeyBase64)
	return err
}

func decodeEd25519PublicKey(publicKeyBase64 string) ([]byte, error) {
	decoded, err := base64.StdEncoding.DecodeString(publicKeyBase64)
	if err != nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("public_key_base64 decode failed: %v", err), err)
	}
	if len(decoded) != 32 {
		return nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("public_key_base64 must decode to exactly 32 bytes, got %d", len(decoded)), nil)
	}
	return decoded, nil
}

func validateIdentityRole(role string) error {
	if role == "" {
		return nil
	}
	switch role {
	case "device", "backend", "hub", "user":
		return nil
	default:
		return invalidProfilePayload(directoryIdentityProfile, "role must be one of device, backend, hub, or user", nil)
	}
}

func firstIdentityString(values map[string]any, keys ...string) string {
	for _, key := range keys {
		value, _ := values[key].(string)
		if strings.TrimSpace(value) != "" {
			return strings.TrimSpace(value)
		}
	}
	return ""
}

func identityOptionalInt64(value any) int64 {
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

func identityOptionalBool(value any, defaultValue bool) bool {
	if typed, ok := value.(bool); ok {
		return typed
	}
	return defaultValue
}

func firstNonZeroIdentityInt(values ...int) int {
	for _, value := range values {
		if value != 0 {
			return value
		}
	}
	return 0
}
