import json
import unittest

from easynet_sdk import (
    CompatibilityCarrierBase,
    CompatibilityChatCompletionRequest,
    CompatibilityClient,
    CompatibilityFileDeleteRequest,
    CompatibilityFileUploadRequest,
    CompatibilityListModelsRequest,
    CompatibilityStreamChatCompletionRequest,
    SDKError,
)


COMPAT_LIST_INVOCATION_JSON = b"""
{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.openai.list_models@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"auth_token": "tok_example"},
  "content_type": "application/json",
  "metadata": {"request_id": "compat-list-models-1", "profile": "compatibility", "system_ability": "openai.list_models", "carrier_owner": "daemon_sdk"}
}
"""

COMPAT_CHAT_INVOCATION_JSON = b"""
{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.openai.chat_completions@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"request": {"model": "easynet:///r/example/ability/alice.codex.chat", "messages": [{"role": "user", "content": "reply with: ok"}]}},
  "content_type": "application/json",
  "metadata": {"request_id": "compat-chat-completion-1", "profile": "compatibility", "system_ability": "openai.chat_completions", "carrier_owner": "daemon_sdk"}
}
"""

MODEL_PAGE_JSON = b"""
{
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
}
"""

CHAT_COMPLETION_JSON = b"""
{
  "profile": "compatibility",
  "kind": "chat_completion",
  "id": "chatcmpl-example",
  "object": "chat.completion",
  "created": 1,
  "model": "easynet:///r/example/ability/alice.codex.chat",
  "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
  "usage": {"prompt_tokens": 3, "completion_tokens": 1, "total_tokens": 4},
  "metadata": {"profile": "compatibility", "source": "openai.chat_completions"}
}
"""

CHAT_STREAM_JSON = b"""
{
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
}
"""


class MemoryCompatibilityTransport:
    def __init__(self) -> None:
        self.seen: dict[str, dict[str, object]] = {}

    def _remember(self, name: str, request_json: bytes) -> None:
        self.seen[name] = json.loads(request_json)

    def build_list_models_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_list_models", request_json)
        return COMPAT_LIST_INVOCATION_JSON

    def build_chat_completion_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_chat", request_json)
        return COMPAT_CHAT_INVOCATION_JSON

    def build_stream_chat_completion_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_stream_chat", request_json)
        return COMPAT_CHAT_INVOCATION_JSON

    def list_models(self, request_json: bytes) -> bytes:
        self._remember("list_models", request_json)
        return MODEL_PAGE_JSON

    def create_chat_completion(self, request_json: bytes) -> bytes:
        self._remember("create_chat", request_json)
        return CHAT_COMPLETION_JSON

    def stream_chat_completion(self, request_json: bytes) -> bytes:
        self._remember("stream_chat", request_json)
        return CHAT_STREAM_JSON


def compat_base() -> CompatibilityCarrierBase:
    return CompatibilityCarrierBase(
        caller_ura="easynet:///r/example/agent/alice.sdk",
        callee_ura="easynet:///r/example/device/dev-a",
        subject_ura="easynet:///r/example/device/dev-a",
        descriptor_version="1.0.0",
        nonce_base64="AQIDBAUGBwgJCgsMDQ4PEA==",
        causal_context={"form": "none"},
        auth_token="tok_example",
        metadata={"request_id": "compat-list-models-1"},
    )


def chat_request() -> dict[str, object]:
    return {
        "model": "easynet:///r/example/ability/alice.codex.chat",
        "messages": [{"role": "user", "content": "reply with: ok"}],
        "temperature": 0.2,
    }


class CompatibilityClientTests(unittest.TestCase):
    def test_builds_openai_invocations(self) -> None:
        transport = MemoryCompatibilityTransport()
        client = CompatibilityClient(transport)

        draft = client.build_list_models_invocation(CompatibilityListModelsRequest(compat_base()))
        self.assertEqual(
            draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.openai.list_models@1.0.0",
        )
        self.assertEqual(transport.seen["build_list_models"]["auth_token"], "tok_example")

        base = compat_base()
        base = CompatibilityCarrierBase(
            base.caller_ura,
            base.callee_ura,
            base.subject_ura,
            base.descriptor_version,
            base.nonce_base64,
            base.causal_context,
            metadata={"request_id": "compat-chat-completion-1"},
        )
        chat_draft = client.build_chat_completion_invocation(
            CompatibilityChatCompletionRequest(base, chat_request())
        )
        self.assertEqual(
            chat_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.openai.chat_completions@1.0.0",
        )

        stream_draft = client.build_stream_chat_completion_invocation(
            CompatibilityStreamChatCompletionRequest(base, chat_request())
        )
        self.assertEqual(
            stream_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.openai.chat_completions@1.0.0",
        )
        self.assertIs(transport.seen["build_stream_chat"]["request"]["stream"], True)

    def test_projects_models_chat_stream_and_files(self) -> None:
        client = CompatibilityClient(MemoryCompatibilityTransport())

        models = client.list_models(CompatibilityListModelsRequest(compat_base()))
        self.assertEqual(len(models.data), 1)
        self.assertEqual(models.data[0].object, "model")

        base = compat_base()
        base = CompatibilityCarrierBase(
            base.caller_ura,
            base.callee_ura,
            base.subject_ura,
            base.descriptor_version,
            base.nonce_base64,
            base.causal_context,
        )
        chat = client.create_chat_completion(CompatibilityChatCompletionRequest(base, chat_request()))
        self.assertEqual(chat.object, "chat.completion")
        self.assertEqual(len(chat.choices), 1)

        stream = client.stream_chat_completion(
            CompatibilityStreamChatCompletionRequest(base, chat_request())
        )
        self.assertTrue(stream.stream)
        self.assertEqual(stream.done_sentinel, "[DONE]")

        file = client.project_file_upload(
            CompatibilityFileUploadRequest(
                id="file-easynet-docs-1",
                file_ref="easynet:///r/example/resource/alice.files/prompt.jsonl",
                owner_ura="easynet:///r/example/agent/alice.sdk",
                filename="prompt.jsonl",
                purpose="batch",
                content_type="application/jsonl",
                content_hash="sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                size_bytes=19,
                created_at=1783094400,
            )
        )
        self.assertEqual(file.bytes, 19)
        self.assertEqual(file.metadata["file_ref"], "easynet:///r/example/resource/alice.files/prompt.jsonl")

        deleted = client.project_file_delete_result(
            CompatibilityFileDeleteRequest(id="file-easynet-docs-1", deleted=True)
        )
        self.assertTrue(deleted.deleted)
        self.assertEqual(deleted.id, "file-easynet-docs-1")

    def test_rejects_invalid_requests(self) -> None:
        client = CompatibilityClient(MemoryCompatibilityTransport())
        with self.assertRaises(SDKError):
            client.build_list_models_invocation(
                CompatibilityListModelsRequest(
                    CompatibilityCarrierBase("", "", "", "", "", {})
                )
            )
        with self.assertRaises(SDKError):
            client.build_chat_completion_invocation(
                CompatibilityChatCompletionRequest(
                    compat_base(),
                    {"model": "gpt-4", "messages": [{"role": "user", "content": "x"}]},
                )
            )
        stream_true = chat_request()
        stream_true["stream"] = True
        with self.assertRaises(SDKError):
            client.build_chat_completion_invocation(
                CompatibilityChatCompletionRequest(compat_base(), stream_true)
            )
        with self.assertRaises(SDKError):
            client.project_file_upload(CompatibilityFileUploadRequest(purpose="batch"))
        with self.assertRaises(SDKError):
            client.project_file_delete_result(
                CompatibilityFileDeleteRequest(id="file-1", deleted=False)
            )


if __name__ == "__main__":
    unittest.main()
