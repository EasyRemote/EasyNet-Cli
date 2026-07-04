package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

type memoryCompatibilityTransport struct {
	listInvocation         string
	chatInvocation         string
	streamChatInvocation   string
	fileUploadInvocation   string
	fileRetrieveInvocation string
	fileDeleteInvocation   string
	modelPage              string
	chatCompletion         string
	chatStream             string
	file                   string
	fileDelete             string
	seen                   map[string]map[string]any
	closeCalls             int
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

func (m *memoryCompatibilityTransport) BuildFileUploadInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_file_upload", requestJSON)
	return []byte(m.fileUploadInvocation), nil
}

func (m *memoryCompatibilityTransport) BuildFileRetrieveInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_file_retrieve", requestJSON)
	return []byte(m.fileRetrieveInvocation), nil
}

func (m *memoryCompatibilityTransport) BuildFileDeleteInvocation(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("build_file_delete", requestJSON)
	return []byte(m.fileDeleteInvocation), nil
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

func (m *memoryCompatibilityTransport) UploadFile(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("upload_file", requestJSON)
	return []byte(m.file), nil
}

func (m *memoryCompatibilityTransport) RetrieveFile(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("retrieve_file", requestJSON)
	return []byte(m.file), nil
}

func (m *memoryCompatibilityTransport) DeleteFile(_ context.Context, requestJSON []byte) ([]byte, error) {
	m.remember("delete_file", requestJSON)
	return []byte(m.fileDelete), nil
}

func (m *memoryCompatibilityTransport) Close(ctx context.Context) error {
	m.closeCalls++
	return nil
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

func compatibilityFileUploadRequest(base CompatibilityCarrierBase) CompatibilityFileUploadRequest {
	base.Metadata = map[string]any{"request_id": "compat-file-upload-1"}
	return CompatibilityFileUploadRequest{
		CompatibilityCarrierBase: base,
		ID:                       "file-easynet-docs-1",
		FileRef:                  "easynet:///r/example/resource/alice.files/prompt.jsonl",
		OwnerURA:                 "easynet:///r/example/agent/alice.sdk",
		Filename:                 "prompt.jsonl",
		Purpose:                  "batch",
		ContentType:              "application/jsonl",
		BytesBase64:              "eyJwcm9tcHQiOiJoaSJ9Cg==",
		ContentHash:              "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		SizeBytes:                19,
		CreatedAt:                1783094400,
		Metadata:                 map[string]any{"file_route": "uploads"},
	}
}

func compatibilityFileRequest(base CompatibilityCarrierBase) CompatibilityFileRequest {
	base.AuthToken = ""
	base.Metadata = map[string]any{"request_id": "compat-file-retrieve-1"}
	return CompatibilityFileRequest{
		CompatibilityCarrierBase: base,
		ID:                       "file-easynet-docs-1",
		FileRef:                  "easynet:///r/example/resource/alice.files/prompt.jsonl",
		Filename:                 "prompt.jsonl",
		Purpose:                  "batch",
		SizeBytes:                19,
		CreatedAt:                1783094400,
	}
}

func compatibilityFileDeleteRequest(base CompatibilityCarrierBase) CompatibilityFileDeleteRequest {
	base.AuthToken = ""
	base.Metadata = map[string]any{"request_id": "compat-file-delete-1"}
	return CompatibilityFileDeleteRequest{
		CompatibilityCarrierBase: base,
		ID:                       "file-easynet-docs-1",
		FileRef:                  "easynet:///r/example/resource/alice.files/prompt.jsonl",
		Deleted:                  true,
	}
}

func TestCompatibilityBuildsOpenAIInvocations(t *testing.T) {
	transport := &memoryCompatibilityTransport{
		listInvocation:         compatibilityListModelsInvocationJSON,
		chatInvocation:         compatibilityChatCompletionInvocationJSON,
		streamChatInvocation:   compatibilityStreamChatCompletionInvocationJSON,
		fileUploadInvocation:   compatibilityFileUploadInvocationJSON,
		fileRetrieveInvocation: compatibilityFileRetrieveInvocationJSON,
		fileDeleteInvocation:   compatibilityFileDeleteInvocationJSON,
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

	uploadDraft, err := client.BuildFileUploadInvocation(context.Background(), compatibilityFileUploadRequest(compatibilityBaseForTest()))
	if err != nil {
		t.Fatal(err)
	}
	if uploadDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.openai.files.upload@1.0.0" {
		t.Fatalf("file upload descriptor = %q", uploadDraft.DescriptorRef())
	}
	if transport.seen["build_file_upload"]["auth_token"] != "tok_example" {
		t.Fatalf("file upload auth token not preserved: %#v", transport.seen["build_file_upload"])
	}
	metadata := transport.seen["build_file_upload"]["metadata"].(map[string]any)
	if metadata["request_id"] != "compat-file-upload-1" || metadata["file_route"] != "uploads" {
		t.Fatalf("file metadata not merged: %#v", metadata)
	}

	retrieveDraft, err := client.BuildFileRetrieveInvocation(context.Background(), compatibilityFileRequest(compatibilityBaseForTest()))
	if err != nil {
		t.Fatal(err)
	}
	if retrieveDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.openai.files.retrieve@1.0.0" {
		t.Fatalf("file retrieve descriptor = %q", retrieveDraft.DescriptorRef())
	}
	getDraft, err := client.BuildFileGetInvocation(context.Background(), compatibilityFileRequest(compatibilityBaseForTest()))
	if err != nil {
		t.Fatal(err)
	}
	if getDraft.DescriptorRef() != retrieveDraft.DescriptorRef() {
		t.Fatalf("file get descriptor = %q", getDraft.DescriptorRef())
	}

	deleteDraft, err := client.BuildFileDeleteInvocation(context.Background(), compatibilityFileDeleteRequest(compatibilityBaseForTest()))
	if err != nil {
		t.Fatal(err)
	}
	if deleteDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.openai.files.delete@1.0.0" {
		t.Fatalf("file delete descriptor = %q", deleteDraft.DescriptorRef())
	}
}

func TestCompatibilityProjectsModelsChatStreamAndFiles(t *testing.T) {
	transport := &memoryCompatibilityTransport{
		modelPage:      compatibilityModelPageJSON,
		chatCompletion: compatibilityChatCompletionJSON,
		chatStream:     compatibilityChatStreamJSON,
		file:           compatibilityFileJSON,
		fileDelete:     compatibilityFileDeleteResultJSON,
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
	chatReq := CompatibilityChatCompletionRequest{CompatibilityCarrierBase: base, Request: compatibilityChatRequest()}
	chat, err := client.ChatCompletions(context.Background(), chatReq)
	if err != nil {
		t.Fatal(err)
	}
	if chat.Object != "chat.completion" || len(chat.Choices) != 1 {
		t.Fatalf("unexpected chat completion: %#v", chat)
	}
	if transport.seen["create_chat"]["request"] == nil {
		t.Fatalf("normative chat method did not delegate to transport: %#v", transport.seen["create_chat"])
	}

	streamReq := CompatibilityStreamChatCompletionRequest{CompatibilityCarrierBase: base, Request: compatibilityChatRequest()}
	stream, err := client.StreamChatCompletions(context.Background(), streamReq)
	if err != nil {
		t.Fatal(err)
	}
	if !stream.Stream || stream.DoneSentinel != "[DONE]" || len(stream.Items) != 1 {
		t.Fatalf("unexpected chat stream: %#v", stream)
	}
	if request := transport.seen["stream_chat"]["request"].(map[string]any); request["stream"] != true {
		t.Fatalf("normative stream method did not force stream=true: %#v", request)
	}

	legacyChat, err := client.CreateChatCompletion(context.Background(), chatReq)
	if err != nil {
		t.Fatal(err)
	}
	if legacyChat.ID != chat.ID {
		t.Fatalf("legacy chat wrapper diverged: %#v vs %#v", legacyChat, chat)
	}
	legacyStream, err := client.StreamChatCompletion(context.Background(), streamReq)
	if err != nil {
		t.Fatal(err)
	}
	if legacyStream.DoneSentinel != stream.DoneSentinel {
		t.Fatalf("legacy stream wrapper diverged: %#v vs %#v", legacyStream, stream)
	}

	uploaded, err := client.UploadFile(context.Background(), compatibilityFileUploadRequest(compatibilityBaseForTest()))
	if err != nil {
		t.Fatal(err)
	}
	if uploaded.ID != "file-easynet-docs-1" || uploaded.Bytes != 19 {
		t.Fatalf("unexpected uploaded file projection: %#v", uploaded)
	}

	retrieved, err := client.RetrieveFile(context.Background(), compatibilityFileRequest(compatibilityBaseForTest()))
	if err != nil {
		t.Fatal(err)
	}
	if retrieved.ID != uploaded.ID || retrieved.Filename != "prompt.jsonl" {
		t.Fatalf("unexpected retrieved file projection: %#v", retrieved)
	}
	got, err := client.GetFile(context.Background(), compatibilityFileRequest(compatibilityBaseForTest()))
	if err != nil {
		t.Fatal(err)
	}
	if got.ID != uploaded.ID || got.Filename != "prompt.jsonl" {
		t.Fatalf("unexpected get file projection: %#v", got)
	}

	daemonDeleted, err := client.DeleteFile(context.Background(), compatibilityFileDeleteRequest(compatibilityBaseForTest()))
	if err != nil {
		t.Fatal(err)
	}
	if !daemonDeleted.Deleted || daemonDeleted.ID != "file-easynet-docs-1" {
		t.Fatalf("unexpected daemon file delete projection: %#v", daemonDeleted)
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
	if _, err := client.BuildFileUploadInvocation(context.Background(), CompatibilityFileUploadRequest{
		CompatibilityCarrierBase: compatibilityBaseForTest(),
		Purpose:                  "batch",
		BytesBase64:              "AQID",
	}); err == nil {
		t.Fatal("expected file bytes without filename rejection")
	}
	if _, err := client.BuildFileUploadInvocation(context.Background(), CompatibilityFileUploadRequest{Purpose: "batch"}); err == nil {
		t.Fatal("expected incomplete file carrier rejection")
	}
	if _, err := client.ProjectFileDeleteResult(CompatibilityFileDeleteRequest{ID: "file-1", Deleted: false}); err == nil {
		t.Fatal("expected non-deleted result rejection")
	}
}

func TestCompatibilityClientCloseDelegatesOnceAndFailsClosed(t *testing.T) {
	transport := &memoryCompatibilityTransport{listInvocation: compatibilityListModelsInvocationJSON}
	client, err := NewCompatibilityClient(transport)
	if err != nil {
		t.Fatal(err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if transport.closeCalls != 1 {
		t.Fatalf("close calls = %d, want 1", transport.closeCalls)
	}
	_, err = client.BuildListModelsInvocation(context.Background(), CompatibilityListModelsRequest{CompatibilityCarrierBase: compatibilityBaseForTest()})
	if err == nil {
		t.Fatalf("BuildListModelsInvocation after close succeeded")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	if len(transport.seen) != 0 {
		t.Fatalf("transport called after close: %#v", transport.seen)
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

const compatibilityFileUploadInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.openai.files.upload@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"file_ref": "easynet:///r/example/resource/alice.files/prompt.jsonl", "purpose": "batch"},
  "content_type": "application/json",
  "metadata": {"request_id": "compat-file-upload-1", "profile": "compatibility", "system_ability": "openai.files.upload", "carrier_owner": "daemon_sdk"}
}`

const compatibilityFileRetrieveInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.openai.files.retrieve@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"file_id": "file-easynet-docs-1"},
  "content_type": "application/json",
  "metadata": {"request_id": "compat-file-retrieve-1", "profile": "compatibility", "system_ability": "openai.files.retrieve", "carrier_owner": "daemon_sdk"}
}`

const compatibilityFileDeleteInvocationJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.openai.files.delete@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"file_id": "file-easynet-docs-1", "deleted": true},
  "content_type": "application/json",
  "metadata": {"request_id": "compat-file-delete-1", "profile": "compatibility", "system_ability": "openai.files.delete", "carrier_owner": "daemon_sdk"}
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

const compatibilityFileJSON = `{
  "profile": "compatibility",
  "kind": "file",
  "id": "file-easynet-docs-1",
  "object": "file",
  "bytes": 19,
  "created_at": 1783094400,
  "filename": "prompt.jsonl",
  "purpose": "batch",
  "status": "processed",
  "metadata": {"profile": "compatibility", "source": "openai.files", "file_ref": "easynet:///r/example/resource/alice.files/prompt.jsonl"}
}`

const compatibilityFileDeleteResultJSON = `{
  "profile": "compatibility",
  "kind": "file_delete_result",
  "id": "file-easynet-docs-1",
  "object": "file",
  "deleted": true,
  "metadata": {"profile": "compatibility", "source": "openai.files.delete"}
}`
