package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

type memoryCompatibilityTransport struct {
	listInvocation       string
	chatInvocation       string
	streamChatInvocation string
	modelPage            string
	chatCompletion       string
	chatStream           string
	seen                 map[string]map[string]any
}

func (m *memoryCompatibilityTransport) remember(name string, requestJSON []byte) {
	if m.seen == nil {
		m.seen = map[string]map[string]any{}
	}
	var decoded map[string]any
	_ = json.Unmarshal(requestJSON, &decoded)
	m.seen[name] = decoded
}

func (m *memoryCompatibilityTransport) BuildListModelsInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_list_models", requestJSON)
	return []byte(m.listInvocation), nil
}

func (m *memoryCompatibilityTransport) BuildChatCompletionInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_chat", requestJSON)
	return []byte(m.chatInvocation), nil
}

func (m *memoryCompatibilityTransport) BuildStreamChatCompletionInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_stream_chat", requestJSON)
	return []byte(m.streamChatInvocation), nil
}

func (m *memoryCompatibilityTransport) ListModels(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("list_models", requestJSON)
	return []byte(m.modelPage), nil
}

func (m *memoryCompatibilityTransport) CreateChatCompletion(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("create_chat", requestJSON)
	return []byte(m.chatCompletion), nil
}

func (m *memoryCompatibilityTransport) StreamChatCompletion(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("stream_chat", requestJSON)
	return []byte(m.chatStream), nil
}

func compatibilityBaseForTest() CompatibilityCarrierBase {
	return CompatibilityCarrierBase{
		CallerURA:         "easynet:///r/example/agent/alice.sdk",
		CalleeURA:         "easynet:///r/example/device/dev-a",
		SubjectURA:        "easynet:///r/example/device/dev-a",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		AuthToken:         "tok_example",
		Metadata:          map[string]any{"request_id": "compat-list-models-1"},
	}
}

func compatibilityChatRequest() map[string]any {
	return map[string]any{
		"model": "easynet:///r/example/ability/alice.codex.chat",
		"messages": []any{
			map[string]any{"role": "user", "content": "reply with: ok"},
		},
		"temperature": 0.2,
	}
}

func TestCompatibilityBuildsOpenAIInvocations(t *testing.T) {
	transport := &memoryCompatibilityTransport{
		listInvocation:       compatibilityListModelsInvocationJSON,
		chatInvocation:       compatibilityChatCompletionInvocationJSON,
		streamChatInvocation: compatibilityStreamChatCompletionInvocationJSON,
	}
	client, err := NewCompatibilityClient(transport)
	if err != nil {
		t.Fatal(err)
	}

	listDraft, err := client.BuildListModelsInvocation(context.Background(), CompatibilityListModelsRequest{CompatibilityCarrierBase: compatibilityBaseForTest()})
	if err != nil {
		t.Fatal(err)
	}
	if listDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.openai.list_models@1.0.0" {
		t.Fatalf("list descriptor = %q", listDraft.DescriptorRef())
	}
	if transport.seen["build_list_models"]["auth_token"] != "tok_example" {
		t.Fatalf("auth token not preserved: %#v", transport.seen["build_list_models"])
	}

	base := compatibilityBaseForTest()
	base.AuthToken = ""
	base.Metadata = map[string]any{"request_id": "compat-chat-completion-1"}
	chatDraft, err := client.BuildChatCompletionInvocation(context.Background(), CompatibilityChatCompletionRequest{
		CompatibilityCarrierBase: base,
		Request:                  compatibilityChatRequest(),
	})
	if err != nil {
		t.Fatal(err)
	}
	if chatDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.openai.chat_completions@1.0.0" {
		t.Fatalf("chat descriptor = %q", chatDraft.DescriptorRef())
	}

	base.Metadata = map[string]any{"request_id": "compat-stream-chat-completion-1"}
	streamDraft, err := client.BuildStreamChatCompletionInvocation(context.Background(), CompatibilityStreamChatCompletionRequest{
		CompatibilityCarrierBase: base,
		Request:                  compatibilityChatRequest(),
	})
	if err != nil {
		t.Fatal(err)
	}
	if streamDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.openai.chat_completions@1.0.0" {
		t.Fatalf("stream descriptor = %q", streamDraft.DescriptorRef())
	}
	request := transport.seen["build_stream_chat"]["request"].(map[string]any)
	if request["stream"] != true {
		t.Fatalf("stream request did not force stream=true: %#v", request)
	}
}

func TestCompatibilityProjectsModelsChatStreamAndFiles(t *testing.T) {
	transport := &memoryCompatibilityTransport{
		modelPage:      compatibilityModelPageJSON,
		chatCompletion: compatibilityChatCompletionJSON,
		chatStream:     compatibilityChatStreamJSON,
	}
	client, err := NewCompatibilityClient(transport)
	if err != nil {
		t.Fatal(err)
	}

	models, err := client.ListModels(context.Background(), CompatibilityListModelsRequest{CompatibilityCarrierBase: compatibilityBaseForTest()})
	if err != nil {
		t.Fatal(err)
	}
	if len(models.Data) != 1 || models.Data[0].AbilityRef == "" {
		t.Fatalf("unexpected model page: %#v", models)
	}

	base := compatibilityBaseForTest()
	base.AuthToken = ""
	chat, err := client.CreateChatCompletion(context.Background(), CompatibilityChatCompletionRequest{CompatibilityCarrierBase: base, Request: compatibilityChatRequest()})
	if err != nil {
		t.Fatal(err)
	}
	if chat.Object != "chat.completion" || len(chat.Choices) != 1 {
		t.Fatalf("unexpected chat completion: %#v", chat)
	}

	stream, err := client.StreamChatCompletion(context.Background(), CompatibilityStreamChatCompletionRequest{CompatibilityCarrierBase: base, Request: compatibilityChatRequest()})
	if err != nil {
		t.Fatal(err)
	}
	if !stream.Stream || stream.DoneSentinel != "[DONE]" || len(stream.Items) != 1 {
		t.Fatalf("unexpected chat stream: %#v", stream)
	}

	file, err := client.ProjectFileUpload(CompatibilityFileUploadRequest{
		ID:          "file-easynet-docs-1",
		FileRef:     "easynet:///r/example/resource/alice.files/prompt.jsonl",
		OwnerURA:    "easynet:///r/example/agent/alice.sdk",
		Filename:    "prompt.jsonl",
		Purpose:     "batch",
		ContentType: "application/jsonl",
		ContentHash: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		SizeBytes:   19,
		CreatedAt:   1783094400,
	})
	if err != nil {
		t.Fatal(err)
	}
	if file.Kind != "file" || file.Bytes != 19 || file.Metadata["file_ref"] == "" {
		t.Fatalf("unexpected file projection: %#v", file)
	}

	deleted, err := client.ProjectFileDeleteResult(CompatibilityFileDeleteRequest{ID: "file-easynet-docs-1", Deleted: true})
	if err != nil {
		t.Fatal(err)
	}
	if !deleted.Deleted || deleted.ID != "file-easynet-docs-1" {
		t.Fatalf("unexpected file delete projection: %#v", deleted)
	}
}

func TestCompatibilityRejectsInvalidRequests(t *testing.T) {
	client, err := NewCompatibilityClient(&memoryCompatibilityTransport{chatInvocation: compatibilityChatCompletionInvocationJSON})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := client.BuildListModelsInvocation(context.Background(), CompatibilityListModelsRequest{}); err == nil {
		t.Fatal("expected incomplete carrier rejection")
	}
	base := compatibilityBaseForTest()
	base.AuthToken = ""
	if _, err := client.BuildChatCompletionInvocation(context.Background(), CompatibilityChatCompletionRequest{
		CompatibilityCarrierBase: base,
		Request: map[string]any{
			"model":    "gpt-4",
			"messages": []any{map[string]any{"role": "user", "content": "x"}},
		},
	}); err == nil {
		t.Fatal("expected provider nickname rejection")
	}
	unaryStream := compatibilityChatRequest()
	unaryStream["stream"] = true
	if _, err := client.BuildChatCompletionInvocation(context.Background(), CompatibilityChatCompletionRequest{CompatibilityCarrierBase: base, Request: unaryStream}); err == nil {
		t.Fatal("expected unary stream=true rejection")
	}
	if _, err := client.ProjectFileUpload(CompatibilityFileUploadRequest{Purpose: "batch"}); err == nil {
		t.Fatal("expected incomplete file facts rejection")
	}
	if _, err := client.ProjectFileDeleteResult(CompatibilityFileDeleteRequest{ID: "file-1", Deleted: false}); err == nil {
		t.Fatal("expected non-deleted result rejection")
	}
}

const compatibilityListModelsInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.openai.list_models@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"auth_token": "tok_example"},
  "content_type": "application/json",
  "metadata": {"request_id": "compat-list-models-1", "profile": "compatibility", "system_ability": "openai.list_models", "carrier_owner": "daemon_sdk"}
}`

const compatibilityChatCompletionInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.openai.chat_completions@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"request": {"model": "easynet:///r/example/ability/alice.codex.chat", "messages": [{"role": "user", "content": "reply with: ok"}], "temperature": 0.2}},
  "content_type": "application/json",
  "metadata": {"request_id": "compat-chat-completion-1", "profile": "compatibility", "system_ability": "openai.chat_completions", "carrier_owner": "daemon_sdk"}
}`

const compatibilityStreamChatCompletionInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.openai.chat_completions@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"request": {"model": "easynet:///r/example/ability/alice.codex.chat", "messages": [{"role": "user", "content": "reply with: ok"}], "stream": true}},
  "content_type": "application/json",
  "metadata": {"request_id": "compat-stream-chat-completion-1", "profile": "compatibility", "system_ability": "openai.chat_completions", "carrier_owner": "daemon_sdk"}
}`

const compatibilityModelPageJSON = `{
  "profile": "compatibility",
  "kind": "model_page",
  "object": "list",
  "data": [{
    "profile": "compatibility",
    "kind": "model",
    "id": "easynet:///r/example/ability/alice.codex.chat",
    "object": "model",
    "created": 0,
    "owned_by": "easynet",
    "ability_ref": "easynet:///r/example/ability/alice.codex.chat",
    "metadata": {"profile": "compatibility", "source": "openai.list_models"}
  }],
  "next_cursor": null,
  "metadata": {"profile": "compatibility", "source": "openai.list_models", "count": 1}
}`

const compatibilityChatCompletionJSON = `{
  "profile": "compatibility",
  "kind": "chat_completion",
  "id": "chatcmpl-example",
  "object": "chat.completion",
  "created": 1,
  "model": "easynet:///r/example/ability/alice.codex.chat",
  "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
  "usage": {"prompt_tokens": 3, "completion_tokens": 1, "total_tokens": 4},
  "metadata": {"profile": "compatibility", "source": "openai.chat_completions"}
}`

const compatibilityChatStreamJSON = `{
  "profile": "compatibility",
  "kind": "chat_completion_stream",
  "stream": true,
  "items": [{
    "profile": "compatibility",
    "kind": "chat_completion_chunk",
    "id": "chatcmpl-example",
    "object": "chat.completion.chunk",
    "created": 1,
    "model": "easynet:///r/example/ability/alice.codex.chat",
    "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}],
    "usage": null,
    "metadata": {"profile": "compatibility", "source": "openai.chat_completions"}
  }],
  "done_sentinel": "[DONE]",
  "metadata": {"profile": "compatibility", "source": "openai.chat_completions"}
}`
