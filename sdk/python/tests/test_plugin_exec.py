from __future__ import annotations

from io import StringIO
import json
import unittest

from easynet_sdk.providers.easynet.plugin_exec import (
    SidecarInvocation,
    SidecarProtocolError,
    serve_exec_plugin,
)


def _frame() -> dict[str, object]:
    return {
        "type": "invoke",
        "call_id": "call-1",
        "invocation": {
            "caller": "easynet:///r/hub/user/alice",
            "callee": "easynet:///r/hub/device/provider",
            "ability": "demo.echo",
            "subject": "easynet:///r/hub/resource/demo",
            "invocation_nonce": [1, 2, 3, 4],
            "causal_context": {"root": True},
            "args": {"message": "hello"},
        },
    }


class PluginExecTests(unittest.TestCase):
    def test_plugin_invocation_projects_daemon_frame(self) -> None:
        invocation = SidecarInvocation.from_frame(_frame())

        self.assertEqual(invocation.call_id, "call-1")
        self.assertEqual(invocation.caller, "easynet:///r/hub/user/alice")
        self.assertEqual(invocation.callee, "easynet:///r/hub/device/provider")
        self.assertEqual(invocation.ability, "demo.echo")
        self.assertEqual(invocation.subject, "easynet:///r/hub/resource/demo")
        self.assertEqual(invocation.invocation_nonce, (1, 2, 3, 4))
        self.assertEqual(invocation.causal_context, {"root": True})
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


if __name__ == "__main__":
    unittest.main()
