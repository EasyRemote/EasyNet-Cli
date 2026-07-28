from __future__ import annotations

from io import StringIO
import json
import unittest

from easynet_sdk.providers.runtime.plugin_exec import (
    SidecarInvocation,
    SidecarProtocolError,
    serve_exec_plugin,
)

CANONICAL_NONCE = list(range(1, 17))


def _frame() -> dict[str, object]:
    return {
        "type": "invoke",
        "call_id": "call-1",
        "invocation": {
            "caller_ura": "easynet:///r/hub/user/alice",
            "callee_ura": "easynet:///r/hub/device/provider",
            "ability_ura": "demo.echo",
            "subject_ura": "easynet:///r/hub/resource/demo",
            "invocation_nonce": CANONICAL_NONCE.copy(),
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
        self.assertEqual(invocation.invocation_nonce, tuple(CANONICAL_NONCE))
        self.assertEqual(invocation.causal_context, {"form": "none"})
        self.assertEqual(invocation.args, {"message": "hello"})

    def test_plugin_invocation_owns_handler_projection(self) -> None:
        frame = _frame()
        invocation_frame = frame["invocation"]
        assert isinstance(invocation_frame, dict)
        invocation_frame["args"] = {"message": "hello", "nested": {"value": "owned"}}

        invocation = SidecarInvocation.from_frame(frame)

        args = invocation_frame["args"]
        assert isinstance(args, dict)
        nested = args["nested"]
        assert isinstance(nested, dict)
        nested["value"] = "mutated-after-projection"

        self.assertEqual(invocation.args["nested"]["value"], "owned")
        with self.assertRaises(TypeError):
            invocation.args["message"] = "handler-mutation"  # type: ignore[index]
        with self.assertRaises(TypeError):
            invocation.args["nested"]["value"] = "handler-mutation"  # type: ignore[index]

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
                "value": {"ok": True, "message": "hello", "nonce_len": 16},
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

    def test_exec_plugin_helper_writes_terminal_error_frame_for_uncorrelated_request(
        self,
    ) -> None:
        output = StringIO()
        frame = _frame()
        del frame["call_id"]

        serve_exec_plugin(
            lambda _invocation: {"ok": True},
            input_stream=StringIO(json.dumps(frame) + "\n"),
            output_stream=output,
        )

        self.assertEqual(
            json.loads(output.getvalue()),
            {
                "type": "error",
                "call_id": "",
                "message": "sidecar frame field 'call_id' must be a string",
            },
        )

    def test_exec_plugin_helper_writes_terminal_error_frame_for_malformed_invocation(
        self,
    ) -> None:
        output = StringIO()
        frame = _frame()
        invocation = frame["invocation"]
        assert isinstance(invocation, dict)
        invocation["descriptor_ref"] = "retired-provider-leak"

        serve_exec_plugin(
            lambda _invocation: {"ok": True},
            input_stream=StringIO(json.dumps(frame) + "\n"),
            output_stream=output,
        )

        response = json.loads(output.getvalue())
        self.assertEqual(response["type"], "error")
        self.assertEqual(response["call_id"], "call-1")
        self.assertIn("canonical invocation frame", response["message"])

    def test_plugin_invocation_rejects_non_invoke_frame(self) -> None:
        frame = _frame()
        frame["type"] = "stream_open"

        with self.assertRaises(SidecarProtocolError):
            SidecarInvocation.from_frame(frame)

    def test_plugin_invocation_rejects_non_canonical_tuple_aliases(self) -> None:
        frame = _frame()
        invocation = frame["invocation"]
        assert isinstance(invocation, dict)
        invocation["caller"] = "easynet:///r/hub/user/bob"

        with self.assertRaisesRegex(SidecarProtocolError, "canonical invocation frame"):
            SidecarInvocation.from_frame(frame)

    def test_plugin_invocation_rejects_unknown_invocation_fields(self) -> None:
        frame = _frame()
        invocation = frame["invocation"]
        assert isinstance(invocation, dict)
        invocation["descriptor_ref"] = "retired-provider-leak"

        with self.assertRaisesRegex(
            SidecarProtocolError, "canonical invocation frame"
        ):
            SidecarInvocation.from_frame(frame)

    def test_plugin_invocation_rejects_unknown_request_fields(self) -> None:
        frame = _frame()
        frame["retired_mode"] = "json"

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

    def test_plugin_invocation_rejects_non_canonical_nonce_length(self) -> None:
        frame = _frame()
        invocation = frame["invocation"]
        assert isinstance(invocation, dict)
        invocation["invocation_nonce"] = [1, 2, 3, 4]

        with self.assertRaisesRegex(SidecarProtocolError, "exactly 16 bytes"):
            SidecarInvocation.from_frame(frame)

    def test_plugin_invocation_rejects_boolean_nonce_bytes(self) -> None:
        frame = _frame()
        invocation = frame["invocation"]
        assert isinstance(invocation, dict)
        invocation["invocation_nonce"] = CANONICAL_NONCE[:-1] + [True]

        with self.assertRaisesRegex(SidecarProtocolError, "must contain bytes"):
            SidecarInvocation.from_frame(frame)


if __name__ == "__main__":
    unittest.main()
