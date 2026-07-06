import json
import unittest

from easynet_sdk import (
    CompatibilityCarrierBase,
    CompatibilityChatCompletionRequest,
    CompatibilityClient,
    ErrorCode,
    CompatibilityFileDeleteRequest,
    CompatibilityFileRequest,
    CompatibilityFileUploadRequest,
    CompatibilityListModelsRequest,
    CompatibilityStreamChatCompletionRequest,
    RuntimeClient,
    RuntimeCompatibilityTransport,
    SDKError,
    is_code,
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

COMPAT_FILE_UPLOAD_INVOCATION_JSON = b"""
{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.openai.files.upload@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"file_ref": "easynet:///r/example/resource/alice.files/prompt.jsonl", "purpose": "batch"},
  "content_type": "application/json",
  "metadata": {"request_id": "compat-file-upload-1", "profile": "compatibility", "system_ability": "openai.files.upload", "carrier_owner": "daemon_sdk"}
}
"""

COMPAT_FILE_RETRIEVE_INVOCATION_JSON = b"""
{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.openai.files.retrieve@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"file_id": "file-easynet-docs-1"},
  "content_type": "application/json",
  "metadata": {"request_id": "compat-file-retrieve-1", "profile": "compatibility", "system_ability": "openai.files.retrieve", "carrier_owner": "daemon_sdk"}
}
"""

COMPAT_FILE_DELETE_INVOCATION_JSON = b"""
{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.openai.files.delete@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"file_id": "file-easynet-docs-1", "deleted": true},
  "content_type": "application/json",
  "metadata": {"request_id": "compat-file-delete-1", "profile": "compatibility", "system_ability": "openai.files.delete", "carrier_owner": "daemon_sdk"}
}
"""

FILE_JSON = b"""
{
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
}
"""

FILE_DELETE_RESULT_JSON = b"""
{
  "profile": "compatibility",
  "kind": "file_delete_result",
  "id": "file-easynet-docs-1",
  "object": "file",
  "deleted": true,
  "metadata": {"profile": "compatibility", "source": "openai.files.delete"}
}
"""


class MemoryCompatibilityTransport:
    def __init__(self) -> None:
        self.seen: dict[str, dict[str, object]] = {}
        self.close_calls = 0

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

    def build_file_upload_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_file_upload", request_json)
        return COMPAT_FILE_UPLOAD_INVOCATION_JSON

    def build_file_retrieve_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_file_retrieve", request_json)
        return COMPAT_FILE_RETRIEVE_INVOCATION_JSON

    def build_file_delete_invocation(self, request_json: bytes) -> bytes:
        self._remember("build_file_delete", request_json)
        return COMPAT_FILE_DELETE_INVOCATION_JSON

    def list_models(self, request_json: bytes) -> bytes:
        self._remember("list_models", request_json)
        return MODEL_PAGE_JSON

    def chat_completions(self, request_json: bytes) -> bytes:
        self._remember("create_chat", request_json)
        return CHAT_COMPLETION_JSON

    def stream_chat_completions(self, request_json: bytes) -> bytes:
        self._remember("stream_chat", request_json)
        return CHAT_STREAM_JSON

    def upload_file(self, request_json: bytes) -> bytes:
        self._remember("upload_file", request_json)
        return FILE_JSON

    def get_file(self, request_json: bytes) -> bytes:
        self._remember("retrieve_file", request_json)
        return FILE_JSON

    def delete_file(self, request_json: bytes) -> bytes:
        self._remember("delete_file", request_json)
        return FILE_DELETE_RESULT_JSON

    def project_model_page(self, models_json: bytes) -> bytes:
        self._remember("project_model_page", models_json)
        return MODEL_PAGE_JSON

    def project_chat_completion(self, completion_json: bytes) -> bytes:
        self._remember("project_chat_completion", completion_json)
        return CHAT_COMPLETION_JSON

    def project_chat_stream(self, stream_json: bytes) -> bytes:
        self._remember("project_chat_stream", stream_json)
        return CHAT_STREAM_JSON

    def project_file_upload(self, file_json: bytes) -> bytes:
        self._remember("project_file_upload", file_json)
        return FILE_JSON

    def project_file(self, file_json: bytes) -> bytes:
        self._remember("project_file", file_json)
        return FILE_JSON

    def project_file_delete_result(self, result_json: bytes) -> bytes:
        self._remember("project_file_delete_result", result_json)
        return FILE_DELETE_RESULT_JSON

    def close(self) -> None:
        self.close_calls += 1


class MemoryCompatibilityRuntimeTransport:
    def __init__(self, output_json: object) -> None:
        self.output_json = output_json
        self.seen_draft: dict[str, object] | None = None
        self.stream_transport = MemoryCompatibilityStreamTransport()
        self.close_calls = 0

    def invoke(self, draft_json: bytes) -> bytes:
        self.seen_draft = json.loads(draft_json)
        return json.dumps(
            {
                "ok": True,
                "tuple": self.seen_draft,
                "terminal_state": "Completed",
                "output_content_type": "application/json",
                "output_json": self.output_json,
                "elapsed_ms": 1,
                "receipt": {},
                "error": None,
            },
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")

    def open_stream(self, draft_json: bytes):
        self.seen_draft = json.loads(draft_json)
        return self.stream_transport, b'{"stream_id":"compat-stream","state":"Open"}'

    def close(self) -> None:
        self.close_calls += 1


class MemoryCompatibilityStreamTransport:
    def __init__(self) -> None:
        self.events = [
            b'{"sequence":1,"kind":"data","state":"Open","payload_json":{"delta":"hel"}}',
            b'{"sequence":2,"kind":"data","state":"Open","payload_json":{"delta":"lo"}}',
            b'{"sequence":3,"kind":"terminal","state":"Completed","terminal":true}',
        ]
        self.close_calls = 0

    def recv(self, timeout: float | None = None) -> bytes:
        return self.events.pop(0)

    def cancel(self, reason: str) -> bytes:
        return b'{"stream_id":"compat-stream","cancelled":true,"state":"Cancelled","terminal":true}'

    def close(self) -> None:
        self.close_calls += 1


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


def file_upload_request(base: CompatibilityCarrierBase) -> CompatibilityFileUploadRequest:
    carrier = CompatibilityCarrierBase(
        base.caller_ura,
        base.callee_ura,
        base.subject_ura,
        base.descriptor_version,
        base.nonce_base64,
        base.causal_context,
        base.auth_token,
        {"request_id": "compat-file-upload-1"},
    )
    return CompatibilityFileUploadRequest(
        id="file-easynet-docs-1",
        file_ref="easynet:///r/example/resource/alice.files/prompt.jsonl",
        owner_ura="easynet:///r/example/agent/alice.sdk",
        filename="prompt.jsonl",
        purpose="batch",
        content_type="application/jsonl",
        content_hash="sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        size_bytes=19,
        created_at=1783094400,
        metadata={"file_route": "uploads"},
        base=carrier,
    )


def file_request(base: CompatibilityCarrierBase) -> CompatibilityFileRequest:
    carrier = CompatibilityCarrierBase(
        base.caller_ura,
        base.callee_ura,
        base.subject_ura,
        base.descriptor_version,
        base.nonce_base64,
        base.causal_context,
        metadata={"request_id": "compat-file-retrieve-1"},
    )
    return CompatibilityFileRequest(
        id="file-easynet-docs-1",
        file_ref="easynet:///r/example/resource/alice.files/prompt.jsonl",
        filename="prompt.jsonl",
        purpose="batch",
        size_bytes=19,
        created_at=1783094400,
        base=carrier,
    )


def file_delete_request(base: CompatibilityCarrierBase) -> CompatibilityFileDeleteRequest:
    carrier = CompatibilityCarrierBase(
        base.caller_ura,
        base.callee_ura,
        base.subject_ura,
        base.descriptor_version,
        base.nonce_base64,
        base.causal_context,
        metadata={"request_id": "compat-file-delete-1"},
    )
    return CompatibilityFileDeleteRequest(
        id="file-easynet-docs-1",
        file_ref="easynet:///r/example/resource/alice.files/prompt.jsonl",
        deleted=True,
        base=carrier,
    )


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

        upload_draft = client.build_file_upload_invocation(file_upload_request(compat_base()))
        self.assertEqual(
            upload_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.openai.files.upload@1.0.0",
        )
        self.assertEqual(transport.seen["build_file_upload"]["auth_token"], "tok_example")
        self.assertEqual(transport.seen["build_file_upload"]["metadata"]["request_id"], "compat-file-upload-1")
        self.assertEqual(transport.seen["build_file_upload"]["metadata"]["file_route"], "uploads")

        retrieve_draft = client.build_file_retrieve_invocation(file_request(compat_base()))
        self.assertEqual(
            retrieve_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.openai.files.retrieve@1.0.0",
        )
        delete_draft = client.build_file_delete_invocation(file_delete_request(compat_base()))
        self.assertEqual(
            delete_draft.descriptor_ref,
            "easynet:///r/example/ability/device.dev-a.openai.files.delete@1.0.0",
        )

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
        chat = client.chat_completions(CompatibilityChatCompletionRequest(base, chat_request()))
        self.assertEqual(chat.object, "chat.completion")
        self.assertEqual(len(chat.choices), 1)

        stream = client.stream_chat_completions(
            CompatibilityStreamChatCompletionRequest(base, chat_request())
        )
        self.assertTrue(stream.stream)
        self.assertEqual(stream.done_sentinel, "[DONE]")

        uploaded = client.upload_file(file_upload_request(compat_base()))
        self.assertEqual(uploaded.id, "file-easynet-docs-1")
        self.assertEqual(uploaded.bytes, 19)

        retrieved = client.get_file(file_request(compat_base()))
        self.assertEqual(retrieved.id, uploaded.id)
        self.assertEqual(retrieved.filename, "prompt.jsonl")

        daemon_deleted = client.delete_file(file_delete_request(compat_base()))
        self.assertTrue(daemon_deleted.deleted)
        self.assertEqual(daemon_deleted.id, "file-easynet-docs-1")

        runtime_carrier = MemoryCompatibilityTransport()
        runtime_transport = MemoryCompatibilityRuntimeTransport(
            {
                "object": "list",
                "data": [{"id": "easynet:///r/example/ability/alice.codex.chat"}],
            }
        )
        runtime_client = CompatibilityClient(
            RuntimeCompatibilityTransport(
                carrier=runtime_carrier,
                runtime=RuntimeClient(runtime_transport),
            )
        )
        runtime_models = runtime_client.list_models(
            CompatibilityListModelsRequest(compat_base())
        )
        self.assertEqual(runtime_models.kind, "model_page")
        assert runtime_transport.seen_draft is not None
        self.assertEqual(
            runtime_transport.seen_draft["metadata"]["system_ability"],
            "openai.list_models",
        )
        self.assertIn("project_model_page", runtime_carrier.seen)

        runtime_stream = runtime_client.stream_chat_completions(
            CompatibilityStreamChatCompletionRequest(base, chat_request())
        )
        self.assertTrue(runtime_stream.stream)
        self.assertEqual(runtime_transport.stream_transport.close_calls, 1)
        self.assertEqual(
            runtime_carrier.seen["project_chat_stream"]["chunks"],
            [{"delta": "hel"}, {"delta": "lo"}],
        )

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
            client.build_file_upload_invocation(CompatibilityFileUploadRequest(purpose="batch"))
        with self.assertRaises(SDKError):
            client.project_file_delete_result(
                CompatibilityFileDeleteRequest(id="file-1", deleted=False)
            )

    def test_close_delegates_once_and_fails_closed(self) -> None:
        transport = MemoryCompatibilityTransport()
        client = CompatibilityClient(transport)

        client.close()
        client.close()

        self.assertEqual(transport.close_calls, 1)
        with self.assertRaises(SDKError) as caught:
            client.build_list_models_invocation(
                CompatibilityListModelsRequest(compat_base())
            )
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(transport.seen, {})


if __name__ == "__main__":
    unittest.main()
