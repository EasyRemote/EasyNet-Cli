package easynet

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
)

const (
	wrapperAbilityFileTransfer          = "wrapper.file.transfer"
	wrapperAbilityTerminalStart         = "wrapper.terminal.start"
	wrapperAbilityRemoteDesktopStart    = "wrapper.remote_desktop.start"
	wrapperAbilityBrowserStart          = "wrapper.browser.start"
	wrapperAbilityMediaStart            = "wrapper.media.start"
	wrapperDefaultFileTransferOperation = "transfer"
)

var wrapperCarrierArgKeys = map[string]struct{}{
	"caller_ura":         {},
	"callee_ura":         {},
	"subject_ura":        {},
	"descriptor_version": {},
	"nonce_base64":       {},
	"causal_context":     {},
	"metadata":           {},
	"ability_name":       {},
}

// WrapperRuntimeTransport lowers convenience wrapper requests into Runtime
// Core invocations and projects daemon facts back into wrapper DTOs.
type WrapperRuntimeTransport struct {
	runtime  *RuntimeClient
	identity *IdentityClient
}

func NewWrapperRuntimeTransport(runtime *RuntimeClient, identity *IdentityClient) (*WrapperRuntimeTransport, error) {
	if runtime == nil {
		return nil, invalidProfileClient(wrappersProfile, "runtime client is required")
	}
	if identity == nil {
		return nil, invalidProfileClient(wrappersProfile, "identity client is required")
	}
	return &WrapperRuntimeTransport{runtime: runtime, identity: identity}, nil
}

func NewRuntimeWrapperClient(runtime *RuntimeClient, identity *IdentityClient) (*WrapperClient, error) {
	transport, err := NewWrapperRuntimeTransport(runtime, identity)
	if err != nil {
		return nil, err
	}
	return NewWrapperClientWithTransport(transport)
}

func (t *WrapperRuntimeTransport) BuildFileTransferInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeWrapperFileTransferForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, requestJSON, request.WrapperCarrierBase, wrapperFileTransferAbility(request))
}

func (t *WrapperRuntimeTransport) BuildTerminalSessionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeWrapperTerminalStartForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, requestJSON, request.WrapperCarrierBase, wrapperAbilityTerminalStart)
}

func (t *WrapperRuntimeTransport) BuildRemoteDesktopSessionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeWrapperRemoteDesktopStartForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, requestJSON, request.WrapperCarrierBase, wrapperAbilityRemoteDesktopStart)
}

func (t *WrapperRuntimeTransport) BuildBrowserSessionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeWrapperBrowserStartForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, requestJSON, request.WrapperCarrierBase, wrapperAbilityBrowserStart)
}

func (t *WrapperRuntimeTransport) BuildMediaSessionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeWrapperMediaStartForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, requestJSON, request.WrapperCarrierBase, wrapperAbilityMediaStart)
}

func (t *WrapperRuntimeTransport) TransferFile(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeWrapperFileTransferForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	output, err := t.invoke(ctx, requestJSON, request.WrapperCarrierBase, wrapperFileTransferAbility(request))
	if err != nil {
		return nil, err
	}
	return projectWrapperFileRecord(output, request)
}

func (t *WrapperRuntimeTransport) StartTerminalSession(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeWrapperTerminalStartForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.invoke(ctx, requestJSON, request.WrapperCarrierBase, wrapperAbilityTerminalStart)
}

func (t *WrapperRuntimeTransport) StartRemoteDesktopSession(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeWrapperRemoteDesktopStartForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.invoke(ctx, requestJSON, request.WrapperCarrierBase, wrapperAbilityRemoteDesktopStart)
}

func (t *WrapperRuntimeTransport) StartBrowserSession(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeWrapperBrowserStartForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.invoke(ctx, requestJSON, request.WrapperCarrierBase, wrapperAbilityBrowserStart)
}

func (t *WrapperRuntimeTransport) StartMediaSession(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeWrapperMediaStartForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.invoke(ctx, requestJSON, request.WrapperCarrierBase, wrapperAbilityMediaStart)
}

func (t *WrapperRuntimeTransport) OpenTerminalSessionStream(ctx context.Context, requestJSON []byte) (*StreamHandle, error) {
	request, err := decodeWrapperTerminalStartForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.openStream(ctx, requestJSON, request.WrapperCarrierBase, wrapperAbilityTerminalStart)
}

func (t *WrapperRuntimeTransport) OpenTerminalSessionBidi(ctx context.Context, requestJSON []byte, streams []BidiStreamDescriptor) (*BidiSession, error) {
	request, err := decodeWrapperTerminalStartForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.openBidi(ctx, requestJSON, request.WrapperCarrierBase, wrapperAbilityTerminalStart, streams)
}

func (t *WrapperRuntimeTransport) OpenRemoteDesktopSessionStream(ctx context.Context, requestJSON []byte) (*StreamHandle, error) {
	request, err := decodeWrapperRemoteDesktopStartForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.openStream(ctx, requestJSON, request.WrapperCarrierBase, wrapperAbilityRemoteDesktopStart)
}

func (t *WrapperRuntimeTransport) OpenRemoteDesktopSessionBidi(ctx context.Context, requestJSON []byte, streams []BidiStreamDescriptor) (*BidiSession, error) {
	request, err := decodeWrapperRemoteDesktopStartForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.openBidi(ctx, requestJSON, request.WrapperCarrierBase, wrapperAbilityRemoteDesktopStart, streams)
}

func (t *WrapperRuntimeTransport) OpenBrowserSessionStream(ctx context.Context, requestJSON []byte) (*StreamHandle, error) {
	request, err := decodeWrapperBrowserStartForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.openStream(ctx, requestJSON, request.WrapperCarrierBase, wrapperAbilityBrowserStart)
}

func (t *WrapperRuntimeTransport) OpenBrowserSessionBidi(ctx context.Context, requestJSON []byte, streams []BidiStreamDescriptor) (*BidiSession, error) {
	request, err := decodeWrapperBrowserStartForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.openBidi(ctx, requestJSON, request.WrapperCarrierBase, wrapperAbilityBrowserStart, streams)
}

func (t *WrapperRuntimeTransport) OpenMediaSessionStream(ctx context.Context, requestJSON []byte) (*StreamHandle, error) {
	request, err := decodeWrapperMediaStartForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.openStream(ctx, requestJSON, request.WrapperCarrierBase, wrapperAbilityMediaStart)
}

func (t *WrapperRuntimeTransport) OpenMediaSessionBidi(ctx context.Context, requestJSON []byte, streams []BidiStreamDescriptor) (*BidiSession, error) {
	request, err := decodeWrapperMediaStartForRuntime(requestJSON)
	if err != nil {
		return nil, err
	}
	return t.openBidi(ctx, requestJSON, request.WrapperCarrierBase, wrapperAbilityMediaStart, streams)
}

func (t *WrapperRuntimeTransport) Close(context.Context) error {
	return nil
}

func (t *WrapperRuntimeTransport) buildInvocationJSON(ctx context.Context, requestJSON []byte, base WrapperCarrierBase, abilityName string) ([]byte, error) {
	draft, err := t.buildInvocation(ctx, requestJSON, base, abilityName)
	if err != nil {
		return nil, err
	}
	raw, err := json.Marshal(draft)
	if err != nil {
		return nil, invalidProfilePayload(wrappersProfile, fmt.Sprintf("encode wrapper invocation: %v", err), err)
	}
	return raw, nil
}

func (t *WrapperRuntimeTransport) buildInvocation(ctx context.Context, requestJSON []byte, base WrapperCarrierBase, abilityName string) (InvocationDraft, error) {
	if t == nil || t.runtime == nil || t.identity == nil {
		return InvocationDraft{}, invalidProfileClient(wrappersProfile, "wrapper runtime transport is not initialized")
	}
	if ctx == nil {
		return InvocationDraft{}, invalidProfileClient(wrappersProfile, "context is required")
	}
	payload, err := wrapperRuntimePayload(requestJSON)
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
		WithJSONArgs(wrapperRuntimeArgs(payload)).
		WithContentType("application/json").
		WithMetadata(wrapperRuntimeMetadata(base.Metadata, abilityName)).
		Build()
}

func (t *WrapperRuntimeTransport) invoke(ctx context.Context, requestJSON []byte, base WrapperCarrierBase, abilityName string) ([]byte, error) {
	draft, err := t.buildInvocation(ctx, requestJSON, base, abilityName)
	if err != nil {
		return nil, err
	}
	result, err := t.runtime.Invoke(ctx, draft)
	if err != nil {
		return nil, err
	}
	if !result.OK() {
		return nil, wrapperInvocationFailureError(result)
	}
	outputJSON := result.OutputJSON()
	if len(outputJSON) == 0 || string(outputJSON) == "null" {
		return nil, invalidProfilePayload(wrappersProfile, "wrapper invocation output_json is required", nil)
	}
	return outputJSON, nil
}

func (t *WrapperRuntimeTransport) openStream(ctx context.Context, requestJSON []byte, base WrapperCarrierBase, abilityName string) (*StreamHandle, error) {
	draft, err := t.buildInvocation(ctx, requestJSON, base, abilityName)
	if err != nil {
		return nil, err
	}
	return t.runtime.InvokeStream(ctx, draft)
}

func (t *WrapperRuntimeTransport) openBidi(ctx context.Context, requestJSON []byte, base WrapperCarrierBase, abilityName string, streams []BidiStreamDescriptor) (*BidiSession, error) {
	draft, err := t.buildInvocation(ctx, requestJSON, base, abilityName)
	if err != nil {
		return nil, err
	}
	return t.runtime.OpenBidi(ctx, draft, streams)
}

func decodeWrapperFileTransferForRuntime(requestJSON []byte) (WrapperFileTransferRequest, error) {
	var req WrapperFileTransferRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return WrapperFileTransferRequest{}, invalidProfilePayload(wrappersProfile, fmt.Sprintf("decode wrapper file-transfer request: %v", err), err)
	}
	if err := validateWrapperFileTransferRequest(req); err != nil {
		return WrapperFileTransferRequest{}, err
	}
	return req, nil
}

func decodeWrapperTerminalStartForRuntime(requestJSON []byte) (WrapperTerminalStartRequest, error) {
	var req WrapperTerminalStartRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return WrapperTerminalStartRequest{}, invalidProfilePayload(wrappersProfile, fmt.Sprintf("decode wrapper terminal request: %v", err), err)
	}
	if err := validateWrapperTerminalStartRequest(req); err != nil {
		return WrapperTerminalStartRequest{}, err
	}
	return req, nil
}

func decodeWrapperRemoteDesktopStartForRuntime(requestJSON []byte) (WrapperRemoteDesktopStartRequest, error) {
	var req WrapperRemoteDesktopStartRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return WrapperRemoteDesktopStartRequest{}, invalidProfilePayload(wrappersProfile, fmt.Sprintf("decode wrapper remote desktop request: %v", err), err)
	}
	if err := validateWrapperRemoteDesktopStartRequest(req); err != nil {
		return WrapperRemoteDesktopStartRequest{}, err
	}
	return req, nil
}

func decodeWrapperBrowserStartForRuntime(requestJSON []byte) (WrapperBrowserStartRequest, error) {
	var req WrapperBrowserStartRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return WrapperBrowserStartRequest{}, invalidProfilePayload(wrappersProfile, fmt.Sprintf("decode wrapper browser request: %v", err), err)
	}
	if err := validateWrapperBrowserStartRequest(req); err != nil {
		return WrapperBrowserStartRequest{}, err
	}
	return req, nil
}

func decodeWrapperMediaStartForRuntime(requestJSON []byte) (WrapperMediaStartRequest, error) {
	var req WrapperMediaStartRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return WrapperMediaStartRequest{}, invalidProfilePayload(wrappersProfile, fmt.Sprintf("decode wrapper media request: %v", err), err)
	}
	if err := validateWrapperMediaStartRequest(req); err != nil {
		return WrapperMediaStartRequest{}, err
	}
	return req, nil
}

func wrapperRuntimePayload(requestJSON []byte) (map[string]any, error) {
	var payload map[string]any
	if err := json.Unmarshal(requestJSON, &payload); err != nil {
		return nil, invalidProfilePayload(wrappersProfile, fmt.Sprintf("decode wrapper request: %v", err), err)
	}
	if payload == nil {
		return nil, invalidProfilePayload(wrappersProfile, "wrapper request must be an object", nil)
	}
	return payload, nil
}

func wrapperRuntimeArgs(payload map[string]any) map[string]any {
	if payload["bytes_b64"] != nil {
		args := map[string]any{}
		for _, key := range []string{"filename", "bytes_b64", "content_type"} {
			if value, ok := payload[key]; ok && !wrapperEmptyArg(value) {
				args[key] = value
			}
		}
		return args
	}
	args := make(map[string]any, len(payload))
	for key, value := range payload {
		if _, carrier := wrapperCarrierArgKeys[key]; carrier {
			continue
		}
		if wrapperEmptyArg(value) {
			continue
		}
		args[key] = value
	}
	return args
}

func wrapperRuntimeMetadata(input map[string]any, abilityName string) map[string]any {
	metadata := map[string]any{}
	for key, value := range input {
		metadata[key] = value
	}
	metadata["profile"] = wrappersProfile
	metadata["system_ability"] = abilityName
	metadata["carrier_owner"] = "daemon_sdk"
	return metadata
}

func wrapperFileTransferAbility(request WrapperFileTransferRequest) string {
	if strings.TrimSpace(request.AbilityName) != "" {
		return strings.TrimSpace(request.AbilityName)
	}
	return wrapperAbilityFileTransfer
}

func wrapperInvocationFailureError(result InvocationResult) error {
	failure := result.Failure()
	message := "wrapper invocation failed"
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
	}, wrappersProfile)
}

func projectWrapperFileRecord(raw []byte, request WrapperFileTransferRequest) ([]byte, error) {
	payload, err := wrapperOutputObject(raw)
	if err != nil {
		return nil, err
	}
	if payload["profile"] == wrappersProfile && payload["kind"] == "file_record" {
		return raw, nil
	}
	fileRef := firstWrapperString(payload, "file_ref", "ura")
	if fileRef == "" {
		return nil, invalidProfilePayload(wrappersProfile, "wrapper file_ref is required", nil)
	}
	contentType := firstNonEmptyWrapper(firstWrapperString(payload, "content_type"), request.ContentType)
	if contentType == "" {
		return nil, invalidProfilePayload(wrappersProfile, "wrapper content_type is required", nil)
	}
	contentHash := wrapperContentHash(firstWrapperString(payload, "content_hash", "sha256"))
	var contentHashPtr *string
	if contentHash != "" {
		contentHashPtr = &contentHash
	}
	sizeBytes := wrapperOptionalInt64(payload["size"])
	if sizeBytes == nil {
		sizeBytes = wrapperOptionalInt64(payload["size_bytes"])
	}
	if sizeBytes == nil {
		sizeBytes = request.SizeBytes
	}
	record := map[string]any{
		"profile":      wrappersProfile,
		"kind":         "file_record",
		"file_ref":     fileRef,
		"owner_ura":    request.OwnerURA,
		"content_type": contentType,
		"size_bytes":   sizeBytes,
		"content_hash": contentHashPtr,
		"metadata": map[string]any{
			"profile":     wrappersProfile,
			"source":      "wrappers.file_transfer",
			"raw_result":  payload,
			"filename":    request.Filename,
			"operation":   firstNonEmptyWrapper(request.Operation, wrapperDefaultFileTransferOperation),
			"ability":     wrapperFileTransferAbility(request),
			"raw_sha256":  firstWrapperString(payload, "sha256"),
			"request_ref": request.FileRef,
		},
	}
	return json.Marshal(record)
}

func wrapperOutputObject(raw []byte) (map[string]any, error) {
	var payload map[string]any
	if err := json.Unmarshal(raw, &payload); err != nil {
		return nil, invalidProfilePayload(wrappersProfile, fmt.Sprintf("decode wrapper output: %v", err), err)
	}
	if payload == nil {
		return nil, invalidProfilePayload(wrappersProfile, "wrapper output must be an object", nil)
	}
	return payload, nil
}

func firstWrapperString(values map[string]any, keys ...string) string {
	for _, key := range keys {
		value, _ := values[key].(string)
		if strings.TrimSpace(value) != "" {
			return strings.TrimSpace(value)
		}
	}
	return ""
}

func wrapperContentHash(value string) string {
	value = strings.TrimSpace(value)
	if value == "" {
		return ""
	}
	if strings.HasPrefix(value, "sha256:") {
		return value
	}
	return "sha256:" + value
}

func wrapperOptionalInt64(value any) *int64 {
	var out int64
	switch typed := value.(type) {
	case int:
		out = int64(typed)
	case int64:
		out = typed
	case float64:
		out = int64(typed)
	case json.Number:
		n, err := typed.Int64()
		if err != nil {
			return nil
		}
		out = n
	default:
		return nil
	}
	return &out
}

func wrapperEmptyArg(value any) bool {
	switch typed := value.(type) {
	case nil:
		return true
	case string:
		return strings.TrimSpace(typed) == ""
	default:
		return false
	}
}
