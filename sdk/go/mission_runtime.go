package easynet

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"strings"
)

const (
	missionAbilityRun    = "mission.run"
	missionAbilityTrack  = "mission.track"
	missionAbilityCancel = "mission.cancel"
	missionAbilityEvents = "mission.events"
)

// MissionRuntimeTransport lowers Mission profile requests into Runtime Core
// invocations and projects daemon Mission facts back into SDK DTOs.
type MissionRuntimeTransport struct {
	runtime  *RuntimeClient
	identity *IdentityClient
}

func NewMissionRuntimeTransport(runtime *RuntimeClient, identity *IdentityClient) (*MissionRuntimeTransport, error) {
	if runtime == nil {
		return nil, invalidProfileClient(missionProfile, "runtime client is required")
	}
	if identity == nil {
		return nil, invalidProfileClient(missionProfile, "identity client is required")
	}
	return &MissionRuntimeTransport{runtime: runtime, identity: identity}, nil
}

func NewRuntimeMissionClient(runtime *RuntimeClient, identity *IdentityClient) (*MissionClient, error) {
	transport, err := NewMissionRuntimeTransport(runtime, identity)
	if err != nil {
		return nil, err
	}
	return NewMissionClient(transport)
}

func (t *MissionRuntimeTransport) BuildRunEALInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeMissionRuntimeRequest[MissionRunRequest](requestJSON, validateMissionRunRequest)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, request.MissionCarrierBase, missionAbilityRun, missionRunArgs(request))
}

func (t *MissionRuntimeTransport) BuildRunFileInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeMissionRuntimeRequest[MissionRunFileRequest](requestJSON, validateMissionRunFileRequest)
	if err != nil {
		return nil, err
	}
	args, err := missionRunFileArgs(request)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, request.MissionCarrierBase, missionAbilityRun, args)
}

func (t *MissionRuntimeTransport) BuildTrackInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeMissionRuntimeRequest[MissionTrackRequest](requestJSON, validateMissionTrackRequest)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, request.MissionCarrierBase, missionAbilityTrack, missionRunIDArgs(request.MissionID))
}

func (t *MissionRuntimeTransport) BuildCancelInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeMissionRuntimeRequest[MissionCancelRequest](requestJSON, validateMissionCancelRequest)
	if err != nil {
		return nil, err
	}
	return t.buildInvocationJSON(ctx, request.MissionCarrierBase, missionAbilityCancel, missionRunIDArgs(request.MissionID))
}

func (t *MissionRuntimeTransport) RunEAL(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeMissionRuntimeRequest[MissionRunRequest](requestJSON, validateMissionRunRequest)
	if err != nil {
		return nil, err
	}
	return t.invokeStatus(ctx, request.MissionCarrierBase, missionAbilityRun, missionRunArgs(request))
}

func (t *MissionRuntimeTransport) RunFile(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeMissionRuntimeRequest[MissionRunFileRequest](requestJSON, validateMissionRunFileRequest)
	if err != nil {
		return nil, err
	}
	args, err := missionRunFileArgs(request)
	if err != nil {
		return nil, err
	}
	return t.invokeStatus(ctx, request.MissionCarrierBase, missionAbilityRun, args)
}

func (t *MissionRuntimeTransport) Track(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeMissionRuntimeRequest[MissionTrackRequest](requestJSON, validateMissionTrackRequest)
	if err != nil {
		return nil, err
	}
	return t.invokeStatus(ctx, request.MissionCarrierBase, missionAbilityTrack, missionRunIDArgs(request.MissionID))
}

func (t *MissionRuntimeTransport) Cancel(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeMissionRuntimeRequest[MissionCancelRequest](requestJSON, validateMissionCancelRequest)
	if err != nil {
		return nil, err
	}
	return t.invokeStatus(ctx, request.MissionCarrierBase, missionAbilityCancel, missionRunIDArgs(request.MissionID))
}

func (t *MissionRuntimeTransport) Events(ctx context.Context, requestJSON []byte) ([]byte, error) {
	request, err := decodeMissionRuntimeRequest[MissionEventListRequest](requestJSON, validateMissionEventListRequest)
	if err != nil {
		return nil, err
	}
	output, err := t.invokeOutput(ctx, request.MissionCarrierBase, missionAbilityEvents, missionEventsArgs(request))
	if err != nil {
		return nil, err
	}
	return missionRuntimeProjectEvents(output, request)
}

func (t *MissionRuntimeTransport) OpenEventStream(ctx context.Context, requestJSON []byte) (*StreamHandle, error) {
	request, err := decodeMissionRuntimeRequest[MissionEventListRequest](requestJSON, validateMissionEventListRequest)
	if err != nil {
		return nil, err
	}
	draft, err := t.buildInvocation(ctx, request.MissionCarrierBase, missionAbilityEvents, missionEventsArgs(request))
	if err != nil {
		return nil, err
	}
	return t.runtime.InvokeStream(ctx, draft)
}

func (t *MissionRuntimeTransport) Close(context.Context) error {
	return nil
}

func (t *MissionRuntimeTransport) buildInvocationJSON(ctx context.Context, base MissionCarrierBase, abilityName string, args any) ([]byte, error) {
	draft, err := t.buildInvocation(ctx, base, abilityName, args)
	if err != nil {
		return nil, err
	}
	raw, err := json.Marshal(draft)
	if err != nil {
		return nil, invalidProfilePayload(missionProfile, fmt.Sprintf("encode mission invocation: %v", err), err)
	}
	return raw, nil
}

func (t *MissionRuntimeTransport) buildInvocation(ctx context.Context, base MissionCarrierBase, abilityName string, args any) (InvocationDraft, error) {
	if t == nil || t.runtime == nil || t.identity == nil {
		return InvocationDraft{}, invalidProfileClient(missionProfile, "mission runtime transport is not initialized")
	}
	if ctx == nil {
		return InvocationDraft{}, invalidProfileClient(missionProfile, "context is required")
	}
	if err := validateMissionCarrierBase(base); err != nil {
		return InvocationDraft{}, err
	}
	descriptorRef, err := t.identity.OwnerAbilityDescriptorRef(ctx, base.CalleeURA, abilityName, base.DescriptorVersion)
	if err != nil {
		return InvocationDraft{}, err
	}
	subjectURA, err := descriptorBoundSubjectURA(ctx, t.identity, base.SubjectURA, abilityName)
	if err != nil {
		return InvocationDraft{}, err
	}
	return NewInvocationBuilder().
		WithCallerURA(base.CallerURA).
		WithCalleeURA(base.CalleeURA).
		WithDescriptorRef(descriptorRef).
		WithSubjectURA(subjectURA).
		WithNonceBase64(base.NonceBase64).
		WithCausalContext(base.CausalContext).
		WithJSONArgs(args).
		WithContentType("application/json").
		WithMetadata(missionRuntimeMetadata(base.Metadata, abilityName)).
		Build()
}

func (t *MissionRuntimeTransport) invokeStatus(ctx context.Context, base MissionCarrierBase, abilityName string, args any) ([]byte, error) {
	output, err := t.invokeOutput(ctx, base, abilityName, args)
	if err != nil {
		return nil, err
	}
	return missionRuntimeProjectStatus(output)
}

func (t *MissionRuntimeTransport) invokeOutput(ctx context.Context, base MissionCarrierBase, abilityName string, args any) ([]byte, error) {
	draft, err := t.buildInvocation(ctx, base, abilityName, args)
	if err != nil {
		return nil, err
	}
	result, err := t.runtime.Invoke(ctx, draft)
	if err != nil {
		return nil, err
	}
	if !result.OK() {
		return nil, missionInvocationFailureError(result)
	}
	outputJSON := result.OutputJSON()
	if len(outputJSON) == 0 || string(outputJSON) == "null" {
		return nil, invalidProfilePayload(missionProfile, "mission invocation output_json is required", nil)
	}
	return outputJSON, nil
}

func decodeMissionRuntimeRequest[T any](requestJSON []byte, validate func(any) error) (T, error) {
	var request T
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		return request, invalidProfilePayload(missionProfile, fmt.Sprintf("decode mission runtime request: %v", err), err)
	}
	if err := validate(request); err != nil {
		return request, err
	}
	return request, nil
}

func missionRunArgs(request MissionRunRequest) map[string]any {
	args := map[string]any{"source": request.Source}
	if request.Label != "" {
		args["label"] = request.Label
	}
	return args
}

func missionRunFileArgs(request MissionRunFileRequest) (map[string]any, error) {
	raw, err := os.ReadFile(request.Path)
	if err != nil {
		return nil, invalidProfilePayload(missionProfile, fmt.Sprintf("read mission source file: %v", err), err)
	}
	source := string(raw)
	if strings.TrimSpace(source) == "" {
		return nil, invalidProfilePayload(missionProfile, "mission source file must not be empty", nil)
	}
	label := request.Label
	if label == "" {
		label = request.Path
	}
	return map[string]any{
		"source": source,
		"label":  label,
	}, nil
}

func missionRunIDArgs(missionID string) map[string]any {
	return map[string]any{"run_id": missionID}
}

func missionEventsArgs(request MissionEventListRequest) map[string]any {
	args := map[string]any{
		"run_id":          request.MissionID,
		"cursor_sequence": request.CursorSequence,
	}
	if request.Limit > 0 {
		args["limit"] = request.Limit
	}
	return args
}

func missionRuntimeMetadata(base map[string]any, abilityName string) map[string]any {
	metadata := copyMap(base)
	if metadata == nil {
		metadata = map[string]any{}
	}
	metadata["profile"] = missionProfile
	metadata["system_ability"] = abilityName
	metadata["carrier_owner"] = "daemon_sdk"
	return metadata
}

func missionRuntimeProjectStatus(raw []byte) ([]byte, error) {
	if _, err := NewMissionStatusFromJSON(raw); err != nil {
		return nil, err
	}
	return raw, nil
}

func missionRuntimeProjectEvents(raw []byte, request MissionEventListRequest) ([]byte, error) {
	page, err := NewMissionEventPageFromJSON(raw)
	if err != nil {
		return nil, err
	}
	if page.MissionID != request.MissionID {
		return nil, invalidProfilePayload(missionProfile, "mission event page mission_id must match request", nil)
	}
	if page.CursorSequence != request.CursorSequence {
		return nil, invalidProfilePayload(missionProfile, "mission event page cursor_sequence must match request", nil)
	}
	return raw, nil
}

func missionInvocationFailureError(result InvocationResult) error {
	failure := result.Failure()
	if failure == nil {
		return transportProfileError(missionProfile, "mission invocation failed", nil)
	}
	return withProfileErrorDetails(&SDKError{
		Code:      runtimeFailureCode(failure.Code(), ErrAdmissionDenied),
		Stage:     failure.Stage(),
		Retry:     RetryNever,
		Retryable: failure.Retryable(),
		Message:   failure.Message(),
	}, missionProfile)
}
