from __future__ import annotations

from io import StringIO
import json
import unittest

from easynet_sdk.providers.runtime.plugin_exec import (
    SidecarInvocation,
    SidecarProtocolError,
    serve_exec_plugin,
)


def _frame() -> dict[str, object]:
    return {
        "type": "invoke",
        "call_id": "call-1",
        "invocation": {
            "caller_ura": "easynet:///r/hub/user/alice",
            "callee_ura": "easynet:///r/hub/device/provider",
            "ability_ura": "demo.echo",
            "subject_ura": "easynet:///r/hub/resource/demo",
            "invocation_nonce": [1, 2, 3, 4],
            "causal_context": {"form": "none"},
            "args": {"message": "hello"},
        },
    }


class PluginExecTests(unittest.TestCase):
    def test_plugin_invocation_projects_daemon_frame(self) -> None:
        invocation = SidecarInvocation.from_frame(_frame())

        self.assertEqual(invocation.call_id, "call-1")
        self.assertEqual(invocation.caller_ura, "easynet:///r/hub/user/alice")
        self.assertEqual(invocation.callee_ura, "easynet:///r/hub/device/provider")
        self.assertEqual(invocation.ability_ura, "demo.echo")
        self.assertEqual(invocation.subject_ura, "easynet:///r/hub/resource/demo")
        self.assertEqual(invocation.invocation_nonce, (1, 2, 3, 4))
        self.assertEqual(invocation.causal_context, {"form": "none"})
        self.assertEqual(invocation.args, {"message": "hello"})

    def test_exec_plugin_helper_writes_result_frame(self) -> None:
        output = StringIO()

        def handle(invocation: SidecarInvocation) -> dict[str, object]:
            return {
                "ok": True,
                "message": invocation.args["message"],
                "nonce_len": len(invocation.invocation_nonce),
            }

        serve_exec_plugin(
            handle,
            input_stream=StringIO(json.dumps(_frame()) + "\n"),
            output_stream=output,
        )

        self.assertEqual(
            json.loads(output.getvalue()),
            {
                "type": "result",
                "call_id": "call-1",
                "value": {"ok": True, "message": "hello", "nonce_len": 4},
            },
        )

    def test_exec_plugin_helper_writes_error_frame_for_handler_failure(self) -> None:
        output = StringIO()

        def handle(_invocation: SidecarInvocation) -> object:
            raise RuntimeError("boom")

        serve_exec_plugin(
            handle,
            input_stream=StringIO(json.dumps(_frame()) + "\n"),
            output_stream=output,
        )

        self.assertEqual(
            json.loads(output.getvalue()),
            {"type": "error", "call_id": "call-1", "message": "boom"},
        )

    def test_plugin_invocation_rejects_non_invoke_frame(self) -> None:
        frame = _frame()
        frame["type"] = "stream_open"

        with self.assertRaises(SidecarProtocolError):
            SidecarInvocation.from_frame(frame)

    def test_plugin_invocation_rejects_retired_tuple_aliases(self) -> None:
        frame = _frame()
        invocation = frame["invocation"]
        assert isinstance(invocation, dict)
        invocation["caller"] = "easynet:///r/hub/user/bob"

        with self.assertRaisesRegex(SidecarProtocolError, "retired"):
            SidecarInvocation.from_frame(frame)

    def test_plugin_invocation_rejects_unknown_invocation_fields(self) -> None:
        frame = _frame()
        invocation = frame["invocation"]
        assert isinstance(invocation, dict)
        invocation["descriptor_ref"] = "legacy-provider-leak"

        with self.assertRaisesRegex(
            SidecarProtocolError, "canonical invocation frame"
        ):
            SidecarInvocation.from_frame(frame)

    def test_plugin_invocation_rejects_unknown_request_fields(self) -> None:
        frame = _frame()
        frame["legacy_mode"] = "json"

        with self.assertRaisesRegex(
            SidecarProtocolError, "canonical request frame"
        ):
            SidecarInvocation.from_frame(frame)

    def test_plugin_invocation_rejects_missing_canonical_invocation_objects(
        self,
    ) -> None:
        for field in ("causal_context", "args"):
            with self.subTest(field=field, mode="missing"):
                frame = _frame()
                invocation = frame["invocation"]
                assert isinstance(invocation, dict)
                del invocation[field]

                with self.assertRaisesRegex(SidecarProtocolError, "required"):
                    SidecarInvocation.from_frame(frame)

            with self.subTest(field=field, mode="null"):
                frame = _frame()
                invocation = frame["invocation"]
                assert isinstance(invocation, dict)
                invocation[field] = None

                with self.assertRaisesRegex(SidecarProtocolError, "object"):
                    SidecarInvocation.from_frame(frame)


if __name__ == "__main__":
    unittest.main()
