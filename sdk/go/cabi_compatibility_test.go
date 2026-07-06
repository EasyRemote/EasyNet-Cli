//go:build easynet_cabi && cgo && !windows

package easynet

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"
)

func TestCABICompatibilityTransportBuildsInvokesAndProjects(t *testing.T) {
	libraryPath := buildFakeCABICompatibilityLibrary(t)
	client, transport, err := NewCABICompatibilityClient(libraryPath, "/tmp/easynet-control.json")
	if err != nil {
		t.Fatalf("NewCABICompatibilityClient: %v", err)
	}
	defer func() {
		if err := transport.Close(context.Background()); err != nil {
			t.Fatalf("Close C ABI compatibility transport: %v", err)
		}
	}()

	listDraft, err := client.BuildListModelsInvocation(context.Background(), CompatibilityListModelsRequest{CompatibilityCarrierBase: compatibilityBaseForTest()})
	if err != nil {
		t.Fatalf("BuildListModelsInvocation: %v", err)
	}
	if listDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.openai.list_models@1.0.0" {
		t.Fatalf("list descriptor_ref = %q", listDraft.DescriptorRef())
	}

	base := compatibilityBaseForTest()
	base.AuthToken = ""
	base.Metadata = map[string]any{"request_id": "compat-stream-chat-completion-1"}
	streamDraft, err := client.BuildStreamChatCompletionInvocation(context.Background(), CompatibilityStreamChatCompletionRequest{
		CompatibilityCarrierBase: base,
		Request:                  compatibilityChatRequest(),
	})
	if err != nil {
		t.Fatalf("BuildStreamChatCompletionInvocation: %v", err)
	}
	if streamDraft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.openai.chat_completions@1.0.0" {
		t.Fatalf("stream descriptor_ref = %q", streamDraft.DescriptorRef())
	}

	models, err := client.ListModels(context.Background(), CompatibilityListModelsRequest{CompatibilityCarrierBase: compatibilityBaseForTest()})
	if err != nil {
		t.Fatalf("ListModels: %v", err)
	}
	if len(models.Data) != 1 || models.Data[0].AbilityRef == "" {
		t.Fatalf("models = %#v", models)
	}

	chat, err := client.ChatCompletions(context.Background(), CompatibilityChatCompletionRequest{
		CompatibilityCarrierBase: base,
		Request:                  compatibilityChatRequest(),
	})
	if err != nil {
		t.Fatalf("ChatCompletions: %v", err)
	}
	if chat.Object != "chat.completion" || len(chat.Choices) != 1 {
		t.Fatalf("chat = %#v", chat)
	}

	stream, err := client.StreamChatCompletions(context.Background(), CompatibilityStreamChatCompletionRequest{
		CompatibilityCarrierBase: base,
		Request:                  compatibilityChatRequest(),
	})
	if err != nil {
		t.Fatalf("StreamChatCompletions: %v", err)
	}
	if !stream.Stream || stream.DoneSentinel != "[DONE]" || len(stream.Items) != 1 {
		t.Fatalf("stream = %#v", stream)
	}

	uploaded, err := client.UploadFile(context.Background(), compatibilityFileUploadRequest(compatibilityBaseForTest()))
	if err != nil {
		t.Fatalf("UploadFile: %v", err)
	}
	if uploaded.ID != "file-easynet-docs-1" || uploaded.Bytes != 19 {
		t.Fatalf("uploaded file = %#v", uploaded)
	}

	retrieved, err := client.GetFile(context.Background(), compatibilityFileRequest(compatibilityBaseForTest()))
	if err != nil {
		t.Fatalf("GetFile: %v", err)
	}
	if retrieved.ID != uploaded.ID || retrieved.Filename != "prompt.jsonl" {
		t.Fatalf("retrieved file = %#v", retrieved)
	}

	deleted, err := client.DeleteFile(context.Background(), compatibilityFileDeleteRequest(compatibilityBaseForTest()))
	if err != nil {
		t.Fatalf("DeleteFile: %v", err)
	}
	if !deleted.Deleted || deleted.ID != "file-easynet-docs-1" {
		t.Fatalf("delete result = %#v", deleted)
	}
}

func TestCABICompatibilityTransportRejectsClosedUse(t *testing.T) {
	libraryPath := buildFakeCABICompatibilityLibrary(t)
	client, transport, err := NewCABICompatibilityClient(libraryPath, "")
	if err != nil {
		t.Fatalf("NewCABICompatibilityClient: %v", err)
	}
	if err := transport.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := transport.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if _, err := client.BuildListModelsInvocation(context.Background(), CompatibilityListModelsRequest{CompatibilityCarrierBase: compatibilityBaseForTest()}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("BuildListModelsInvocation after close error = %v, want %s", err, ErrInvalidArgument)
	}
}

func buildFakeCABICompatibilityLibrary(t *testing.T) string {
	t.Helper()
	cc, err := exec.LookPath("cc")
	if err != nil {
		t.Skip("cc is required for C ABI dynamic-library test")
	}
	dir := t.TempDir()
	source := filepath.Join(dir, "fake_easynet_cli_compatibility.c")
	output := filepath.Join(dir, "libeasynet_cli.so")
	args := []string{"-shared", "-fPIC", "-o", output, source}
	if runtime.GOOS == "darwin" {
		output = filepath.Join(dir, "libeasynet_cli.dylib")
		args = []string{"-dynamiclib", "-o", output, source}
	}
	if err := os.WriteFile(source, []byte(fakeCABICompatibilitySource), 0o600); err != nil {
		t.Fatalf("write fake C ABI compatibility source: %v", err)
	}
	cmd := exec.Command(cc, args...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Skipf("build fake C ABI compatibility library: %v\n%s", err, out)
	}
	return output
}

const fakeCABICompatibilitySource = `
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

static char *dup_json(const char *s) {
	size_t n = strlen(s);
	char *out = (char *)malloc(n + 1);
	if (out == 0) return 0;
	memcpy(out, s, n + 1);
	return out;
}

uint32_t easynet_abi_version(void) { return 4u; }
void easynet_string_free(char *s) { free(s); }
int32_t easynet_last_error_json(char **out_error_json) {
	*out_error_json = dup_json("{\"code\":\"GENERIC\",\"stage\":\"fake\",\"message\":\"fake C ABI compatibility error\",\"retry\":\"never\",\"details\":{}}");
	return 0;
}
int32_t easynet_init(const char *control_path, uint64_t *out_handle) {
	if (control_path != 0 && strstr(control_path, "control.json") == 0) return 10;
	*out_handle = 1201;
	return 0;
}
int32_t easynet_shutdown(uint64_t handle) { (void)handle; return 0; }
int32_t easynet_invocation_invoke(uint64_t handle, const char *invocation_json, char **out_result_json) {
	(void)handle;
	if (strstr(invocation_json, "openai.list_models") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"models\":[{\"id\":\"easynet:///r/example/ability/alice.codex.chat\"}]},\"error\":null}");
		return 0;
	}
	if (strstr(invocation_json, "openai.chat_completions") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"id\":\"chatcmpl-example\",\"model\":\"easynet:///r/example/ability/alice.codex.chat\"},\"error\":null}");
		return 0;
	}
	if (strstr(invocation_json, "openai.files.upload") != 0 || strstr(invocation_json, "openai.files.retrieve") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"id\":\"file-easynet-docs-1\",\"filename\":\"prompt.jsonl\"},\"error\":null}");
		return 0;
	}
	if (strstr(invocation_json, "openai.files.delete") != 0) {
		*out_result_json = dup_json("{\"ok\":true,\"tuple\":{},\"terminal_state\":\"Completed\",\"output_json\":{\"id\":\"file-easynet-docs-1\",\"deleted\":true},\"error\":null}");
		return 0;
	}
	return 10;
}
int32_t easynet_compatibility_build_list_models_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle; (void)request_json;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.openai.list_models@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"auth_token\":\"tok_example\"},\"content_type\":\"application/json\",\"metadata\":{\"request_id\":\"compat-list-models-1\",\"profile\":\"compatibility\",\"system_ability\":\"openai.list_models\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_compatibility_build_chat_completion_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "alice.codex.chat") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.openai.chat_completions@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"request\":{\"model\":\"easynet:///r/example/ability/alice.codex.chat\",\"messages\":[{\"role\":\"user\",\"content\":\"reply with: ok\"}],\"temperature\":0.2}},\"content_type\":\"application/json\",\"metadata\":{\"request_id\":\"compat-chat-completion-1\",\"profile\":\"compatibility\",\"system_ability\":\"openai.chat_completions\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_compatibility_build_stream_chat_completion_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "\"stream\":true") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.openai.chat_completions@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"request\":{\"model\":\"easynet:///r/example/ability/alice.codex.chat\",\"messages\":[{\"role\":\"user\",\"content\":\"reply with: ok\"}],\"stream\":true}},\"content_type\":\"application/json\",\"metadata\":{\"request_id\":\"compat-stream-chat-completion-1\",\"profile\":\"compatibility\",\"system_ability\":\"openai.chat_completions\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_compatibility_build_file_upload_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "prompt.jsonl") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.openai.files.upload@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"file_ref\":\"easynet:///r/example/resource/alice.files/prompt.jsonl\",\"purpose\":\"batch\"},\"content_type\":\"application/json\",\"metadata\":{\"request_id\":\"compat-file-upload-1\",\"profile\":\"compatibility\",\"system_ability\":\"openai.files.upload\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_compatibility_build_file_retrieve_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "file-easynet-docs-1") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.openai.files.retrieve@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"file_id\":\"file-easynet-docs-1\"},\"content_type\":\"application/json\",\"metadata\":{\"request_id\":\"compat-file-retrieve-1\",\"profile\":\"compatibility\",\"system_ability\":\"openai.files.retrieve\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_compatibility_build_file_delete_invocation(uint64_t handle, const char *request_json, char **out_invocation_json) {
	(void)handle;
	if (strstr(request_json, "file-easynet-docs-1") == 0) return 10;
	*out_invocation_json = dup_json("{\"caller_ura\":\"easynet:///r/example/agent/alice.sdk\",\"callee_ura\":\"easynet:///r/example/device/dev-a\",\"descriptor_ref\":\"easynet:///r/example/ability/device.dev-a.openai.files.delete@1.0.0\",\"subject_ura\":\"easynet:///r/example/device/dev-a\",\"nonce_base64\":\"AQIDBAUGBwgJCgsMDQ4PEA==\",\"causal_context\":{\"form\":\"none\"},\"args\":{\"file_id\":\"file-easynet-docs-1\",\"deleted\":true},\"content_type\":\"application/json\",\"metadata\":{\"request_id\":\"compat-file-delete-1\",\"profile\":\"compatibility\",\"system_ability\":\"openai.files.delete\",\"carrier_owner\":\"daemon_sdk\"}}");
	return 0;
}
int32_t easynet_compatibility_project_model_page(uint64_t handle, const char *models_json, char **out_models_json) {
	(void)handle; (void)models_json;
	*out_models_json = dup_json("{\"profile\":\"compatibility\",\"kind\":\"model_page\",\"object\":\"list\",\"data\":[{\"profile\":\"compatibility\",\"kind\":\"model\",\"id\":\"easynet:///r/example/ability/alice.codex.chat\",\"object\":\"model\",\"created\":0,\"owned_by\":\"easynet\",\"ability_ref\":\"easynet:///r/example/ability/alice.codex.chat\",\"metadata\":{\"profile\":\"compatibility\",\"source\":\"openai.list_models\"}}],\"next_cursor\":null,\"metadata\":{\"profile\":\"compatibility\",\"source\":\"openai.list_models\",\"count\":1}}");
	return 0;
}
int32_t easynet_compatibility_project_chat_completion(uint64_t handle, const char *completion_json, char **out_completion_json) {
	(void)handle; (void)completion_json;
	*out_completion_json = dup_json("{\"profile\":\"compatibility\",\"kind\":\"chat_completion\",\"id\":\"chatcmpl-example\",\"object\":\"chat.completion\",\"created\":1,\"model\":\"easynet:///r/example/ability/alice.codex.chat\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"ok\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1,\"total_tokens\":4},\"metadata\":{\"profile\":\"compatibility\",\"source\":\"openai.chat_completions\"}}");
	return 0;
}
int32_t easynet_compatibility_project_chat_stream(uint64_t handle, const char *stream_json, char **out_stream_json) {
	(void)handle; (void)stream_json;
	*out_stream_json = dup_json("{\"profile\":\"compatibility\",\"kind\":\"chat_completion_stream\",\"stream\":true,\"items\":[{\"profile\":\"compatibility\",\"kind\":\"chat_completion_chunk\",\"id\":\"chatcmpl-example\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"easynet:///r/example/ability/alice.codex.chat\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}],\"usage\":null,\"metadata\":{\"profile\":\"compatibility\",\"source\":\"openai.chat_completions\"}}],\"done_sentinel\":\"[DONE]\",\"metadata\":{\"profile\":\"compatibility\",\"source\":\"openai.chat_completions\"}}");
	return 0;
}
int32_t easynet_compatibility_project_file_upload(uint64_t handle, const char *file_json, char **out_file_json) {
	(void)handle; (void)file_json;
	*out_file_json = dup_json("{\"profile\":\"compatibility\",\"kind\":\"file\",\"id\":\"file-easynet-docs-1\",\"object\":\"file\",\"bytes\":19,\"created_at\":1783094400,\"filename\":\"prompt.jsonl\",\"purpose\":\"batch\",\"status\":\"processed\",\"metadata\":{\"profile\":\"compatibility\",\"source\":\"openai.files\",\"file_ref\":\"easynet:///r/example/resource/alice.files/prompt.jsonl\"}}");
	return 0;
}
int32_t easynet_compatibility_project_file(uint64_t handle, const char *file_json, char **out_file_json) {
	(void)handle; (void)file_json;
	*out_file_json = dup_json("{\"profile\":\"compatibility\",\"kind\":\"file\",\"id\":\"file-easynet-docs-1\",\"object\":\"file\",\"bytes\":19,\"created_at\":1783094400,\"filename\":\"prompt.jsonl\",\"purpose\":\"batch\",\"status\":\"processed\",\"metadata\":{\"profile\":\"compatibility\",\"source\":\"openai.files\",\"file_ref\":\"easynet:///r/example/resource/alice.files/prompt.jsonl\"}}");
	return 0;
}
int32_t easynet_compatibility_project_file_delete_result(uint64_t handle, const char *result_json, char **out_result_json) {
	(void)handle; (void)result_json;
	*out_result_json = dup_json("{\"profile\":\"compatibility\",\"kind\":\"file_delete_result\",\"id\":\"file-easynet-docs-1\",\"object\":\"file\",\"deleted\":true,\"metadata\":{\"profile\":\"compatibility\",\"source\":\"openai.files.delete\"}}");
	return 0;
}
`
