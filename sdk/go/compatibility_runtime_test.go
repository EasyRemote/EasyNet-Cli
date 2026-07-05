package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"testing"
)

type compatibilityRuntimeIdentityTransport struct {
	seenBuildURA         []map[string]any
	seenBuildDescriptor  []map[string]any
	abilityByName        map[string]string
	descriptorByAbility  map[string]string
	descriptorProjection string
}

func (t *compatibilityRuntimeIdentityTransport) ProjectDescriptorRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return []byte(t.descriptorProjection), nil
}

func (t *compatibilityRuntimeIdentityTransport) BuildDescriptorRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	var req DescriptorRefBuildRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return nil, err
	}
	t.seenBuildDescriptor = append(t.seenBuildDescriptor, requestMapForTest(requestJSON))
	descriptorRef := t.descriptorByAbility[req.AbilityURA]
	if descriptorRef == "" {
		return nil, fmt.Errorf("unexpected ability: %s", req.AbilityURA)
	}
	return []byte(fmt.Sprintf(`{
		"kind":"descriptor_ref",
		"valid":true,
		"descriptor_ref":%q,
		"ability_ura":%q,
		"descriptor_version":%q,
		"profile":"easynet-strict-v2",
		"components":{"owner_ura":"easynet:///r/example/device/dev-a"},
		"metadata":{"grammar_owner":"axon"}
	}`, descriptorRef, req.AbilityURA, req.DescriptorVersion)), nil
}

func (t *compatibilityRuntimeIdentityTransport) ProjectIdentity(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return nil, fmt.Errorf("ProjectIdentity should not be called")
}

func (t *compatibilityRuntimeIdentityTransport) BuildURA(ctx context.Context, requestJSON []byte) ([]byte, error) {
	var req URABuildRequest
	if err := json.Unmarshal(requestJSON, &req); err != nil {
		return nil, err
	}
	t.seenBuildURA = append(t.seenBuildURA, requestMapForTest(requestJSON))
	abilityURA := t.abilityByName[req.AbilityName]
	if abilityURA == "" {
		return nil, fmt.Errorf("unexpected ability name: %s", req.AbilityName)
	}
	return []byte(fmt.Sprintf(`{
		"kind":"ability",
		"valid":true,
		"ura":%q,
		"profile":"easynet-strict-v2",
		"components":{"owner_ura":%q},
		"metadata":{"grammar_owner":"axon"}
	}`, abilityURA, req.OwnerURA)), nil
}

func (t *compatibilityRuntimeIdentityTransport) BuildResourceRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return nil, fmt.Errorf("BuildResourceRef should not be called")
}

func (t *compatibilityRuntimeIdentityTransport) RegisterSigningKey(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return nil, fmt.Errorf("RegisterSigningKey should not be called")
}

func (t *compatibilityRuntimeIdentityTransport) ListSigningKeys(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return nil, fmt.Errorf("ListSigningKeys should not be called")
}

func (t *compatibilityRuntimeIdentityTransport) RevokeSigningKey(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return nil, fmt.Errorf("RevokeSigningKey should not be called")
}

func (t *compatibilityRuntimeIdentityTransport) Signer(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return nil, fmt.Errorf("Signer should not be called")
}

type compatibilityRuntimeInvokeTransport struct {
	outputJSON       string
	fail             bool
	streamTransport  StreamTransport
	streamOpenJSON   []byte
	seenDraft        map[string]any
	seenStreamDraft  map[string]any
	openStreamCalled bool
}

func (t *compatibilityRuntimeInvokeTransport) Invoke(ctx context.Context, draftJSON []byte) ([]byte, error) {
	t.seenDraft = requestMapForTest(draftJSON)
	if t.fail {
		return []byte(fmt.Sprintf(`{
			"ok": false,
			"tuple": %s,
			"terminal_state": "Failed",
			"output_content_type": "application/json",
			"output_json": null,
			"elapsed_ms": 4,
			"receipt": null,
			"error": {
				"code": "ABILITY_FAILED",
				"stage": "execute",
				"message": "compatibility ability rejected request",
				"retryable": false
			}
		}`, draftJSON)), nil
	}
	return []byte(fmt.Sprintf(`{
		"ok": true,
		"tuple": %s,
		"terminal_state": "Completed",
		"output_content_type": "application/json",
		"output_json": %s,
		"selected_node_id": "device-dev-a",
		"scheduling_reason": "direct",
		"elapsed_ms": 7,
		"receipt": {"receipt_id": "compatibility-runtime-1"},
		"error": null
	}`, draftJSON, t.outputJSON)), nil
}

func (t *compatibilityRuntimeInvokeTransport) OpenStream(ctx context.Context, draftJSON []byte) (StreamTransport, []byte, error) {
	t.openStreamCalled = true
	t.seenStreamDraft = requestMapForTest(draftJSON)
	if t.streamTransport == nil {
		return nil, nil, fmt.Errorf("OpenStream should not be called")
	}
	openJSON := t.streamOpenJSON
	if len(openJSON) == 0 {
		openJSON = []byte(`{"stream_id":"runtime-stream-1","state":"Open","max_buffered_events":16}`)
	}
	return t.streamTransport, openJSON, nil
}

func (t *compatibilityRuntimeInvokeTransport) OpenBidi(ctx context.Context, draftJSON []byte, streamsJSON []byte) (BidiTransport, []byte, error) {
	return nil, nil, fmt.Errorf("OpenBidi should not be called")
}

func (t *compatibilityRuntimeInvokeTransport) Prepare(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
	return nil, fmt.Errorf("Prepare should not be called")
}

func (t *compatibilityRuntimeInvokeTransport) SubmitSigned(ctx context.Context, signedJSON []byte) ([]byte, error) {
	return nil, fmt.Errorf("SubmitSigned should not be called")
}

func (t *compatibilityRuntimeInvokeTransport) AwaitHandle(ctx context.Context, handleID uint64) ([]byte, error) {
	return nil, fmt.Errorf("AwaitHandle should not be called")
}

func (t *compatibilityRuntimeInvokeTransport) CancelHandle(ctx context.Context, handleID uint64, reason string) ([]byte, error) {
	return nil, fmt.Errorf("CancelHandle should not be called")
}

func (t *compatibilityRuntimeInvokeTransport) HandleEvents(ctx context.Context, handleID uint64) ([]byte, error) {
	return nil, fmt.Errorf("HandleEvents should not be called")
}

func (t *compatibilityRuntimeInvokeTransport) FreeHandle(ctx context.Context, handleID uint64) error {
	return fmt.Errorf("FreeHandle should not be called")
}

func (t *compatibilityRuntimeInvokeTransport) Close(ctx context.Context) error {
	return nil
}

func TestCompatibilityRuntimeTransportBuildsCompleteInvocation(t *testing.T) {
	identityTransport := newCompatibilityRuntimeIdentityTransport()
	identity, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{outputJSON: compatibilityModelPageJSON})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeCompatibilityClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeCompatibilityClient: %v", err)
	}

	draft, err := client.BuildListModelsInvocation(context.Background(), CompatibilityListModelsRequest{CompatibilityCarrierBase: compatibilityBaseForTest()})
	if err != nil {
		t.Fatalf("BuildListModelsInvocation: %v", err)
	}
	if draft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.openai.list_models@1.0.0" {
		t.Fatalf("descriptor ref = %q", draft.DescriptorRef())
	}
	args := draft.JSONArgs().(map[string]any)
	if args["auth_token"] != "tok_example" {
		t.Fatalf("auth token not carried as ability arg: %#v", args)
	}
	if _, ok := args["caller_ura"]; ok {
		t.Fatalf("carrier leaked into args: %#v", args)
	}
	metadata := draft.Metadata()
	if metadata["request_id"] != "compat-list-models-1" ||
		metadata["profile"] != compatibilityProfile ||
		metadata["system_ability"] != compatibilityAbilityListModels ||
		metadata["carrier_owner"] != "daemon_sdk" {
		t.Fatalf("metadata not normalized: %#v", metadata)
	}
	if len(identityTransport.seenBuildURA) != 1 || identityTransport.seenBuildURA[0]["ability_name"] != compatibilityAbilityListModels {
		t.Fatalf("ability URA was not delegated through identity client: %#v", identityTransport.seenBuildURA)
	}
	if len(identityTransport.seenBuildDescriptor) != 1 || identityTransport.seenBuildDescriptor[0]["descriptor_version"] != "1.0.0" {
		t.Fatalf("descriptor ref was not delegated through identity client: %#v", identityTransport.seenBuildDescriptor)
	}
}

func TestCompatibilityRuntimeTransportInvokesAndProjectsOutput(t *testing.T) {
	identityTransport := newCompatibilityRuntimeIdentityTransport()
	identity, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeTransport := &compatibilityRuntimeInvokeTransport{outputJSON: compatibilityChatCompletionJSON}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeCompatibilityClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeCompatibilityClient: %v", err)
	}

	base := compatibilityBaseForTest()
	base.AuthToken = ""
	base.Metadata = map[string]any{"request_id": "compat-chat-completion-1"}
	completion, err := client.ChatCompletions(context.Background(), CompatibilityChatCompletionRequest{
		CompatibilityCarrierBase: base,
		Request:                  compatibilityChatRequest(),
	})
	if err != nil {
		t.Fatalf("ChatCompletions: %v", err)
	}
	if completion.ID != "chatcmpl-example" || completion.Model != "easynet:///r/example/ability/alice.codex.chat" {
		t.Fatalf("unexpected completion: %#v", completion)
	}
	args := runtimeTransport.seenDraft["args"].(map[string]any)
	request := args["request"].(map[string]any)
	if request["model"] != "easynet:///r/example/ability/alice.codex.chat" {
		t.Fatalf("chat request not sent as ability args: %#v", args)
	}
	metadata := runtimeTransport.seenDraft["metadata"].(map[string]any)
	if metadata["system_ability"] != compatibilityAbilityChatCompletions {
		t.Fatalf("runtime draft metadata not normalized: %#v", metadata)
	}
}

func TestCompatibilityRuntimeTransportMapsTerminalFailure(t *testing.T) {
	identity, err := NewIdentityClient(newCompatibilityRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{fail: true})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeCompatibilityClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeCompatibilityClient: %v", err)
	}

	_, err = client.ListModels(context.Background(), CompatibilityListModelsRequest{CompatibilityCarrierBase: compatibilityBaseForTest()})
	if err == nil {
		t.Fatalf("ListModels succeeded, want failure")
	}
	if !IsCode(err, ErrAbilityFailed) {
		t.Fatalf("error code = %v, want %s", err, ErrAbilityFailed)
	}
	var sdkErr *SDKError
	if !asSDKErrorForTest(err, &sdkErr) || sdkErr.Stage != "execute" || sdkErr.Details["terminal_state"] != "Failed" {
		t.Fatalf("terminal failure details not preserved: %#v", err)
	}
}

func newCompatibilityRuntimeIdentityTransport() *compatibilityRuntimeIdentityTransport {
	return &compatibilityRuntimeIdentityTransport{
		abilityByName: map[string]string{
			compatibilityAbilityListModels:      "easynet:///r/example/ability/device.dev-a.openai.list_models",
			compatibilityAbilityChatCompletions: "easynet:///r/example/ability/device.dev-a.openai.chat_completions",
			compatibilityAbilityFileUpload:      "easynet:///r/example/ability/device.dev-a.openai.files.upload",
			compatibilityAbilityFileRetrieve:    "easynet:///r/example/ability/device.dev-a.openai.files.retrieve",
			compatibilityAbilityFileDelete:      "easynet:///r/example/ability/device.dev-a.openai.files.delete",
		},
		descriptorByAbility: map[string]string{
			"easynet:///r/example/ability/device.dev-a.openai.list_models":      "easynet:///r/example/ability/device.dev-a.openai.list_models@1.0.0",
			"easynet:///r/example/ability/device.dev-a.openai.chat_completions": "easynet:///r/example/ability/device.dev-a.openai.chat_completions@1.0.0",
			"easynet:///r/example/ability/device.dev-a.openai.files.upload":     "easynet:///r/example/ability/device.dev-a.openai.files.upload@1.0.0",
			"easynet:///r/example/ability/device.dev-a.openai.files.retrieve":   "easynet:///r/example/ability/device.dev-a.openai.files.retrieve@1.0.0",
			"easynet:///r/example/ability/device.dev-a.openai.files.delete":     "easynet:///r/example/ability/device.dev-a.openai.files.delete@1.0.0",
		},
		descriptorProjection: identityDescriptorProjectionJSON,
	}
}

func requestMapForTest(raw []byte) map[string]any {
	var value map[string]any
	if err := json.Unmarshal(raw, &value); err != nil {
		panic(err)
	}
	return value
}

func asSDKErrorForTest(err error, target **SDKError) bool {
	return errors.As(err, target)
}
