from __future__ import annotations

from collections.abc import Iterator, Mapping
from io import StringIO
import json
import unittest

from easynet_sdk.providers.runtime.plugin_exec import (
    SidecarInvocation,
    SidecarProtocolError,
    serve_bidi_plugin,
    serve_exec_plugin,
    serve_plugin,
    serve_stream_plugin,
)

CANONICAL_NONCE = list(range(1, 17))


def _frame() -> dict[str, object]:
    return {
        "type": "invoke",
        "call_id": "call-1",
        "invocation": {
            "caller_ura": "easynet:///r/hub/user/alice",
            "callee_ura": "easynet:///r/hub/service/alice.provider",
            "ability_ura": "demo.echo",
            "subject_ura": "easynet:///r/hub/resource/demo",
            "invocation_nonce": CANONICAL_NONCE.copy(),
            "causal_context": {"form": "none"},
            "args": {"message": "hello"},
        },
    }


def _open_frame(frame_type: str = "stream_open") -> dict[str, object]:
    frame = _frame()
    frame["type"] = frame_type
    return frame


class PluginExecTests(unittest.TestCase):
    def test_plugin_invocation_projects_daemon_frame(self) -> None:
        invocation = SidecarInvocation.from_frame(_frame())

        self.assertEqual(invocation.call_id, "call-1")
        self.assertEqual(invocation.caller_ura, "easynet:///r/hub/user/alice")
        self.assertEqual(invocation.callee_ura, "easynet:///r/hub/service/alice.provider")
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

    def test_stream_plugin_helper_writes_items_and_single_terminal(self) -> None:
        output = StringIO()

        def handle(invocation: SidecarInvocation) -> list[dict[str, object]]:
            return [
                {"kind": "opened", "ability_ura": invocation.ability_ura},
                {"kind": "payload", "message": invocation.args["message"]},
            ]

        serve_stream_plugin(
            handle,
            input_stream=StringIO(json.dumps(_open_frame("stream_open")) + "\n"),
            output_stream=output,
            terminal_reason="done",
        )

        frames = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(
            frames,
            [
                {
                    "type": "stream_item",
                    "call_id": "call-1",
                    "value": {"kind": "opened", "ability_ura": "demo.echo"},
                },
                {
                    "type": "stream_item",
                    "call_id": "call-1",
                    "value": {"kind": "payload", "message": "hello"},
                },
                {"type": "terminal", "call_id": "call-1", "reason": "done"},
            ],
        )

    def test_bidi_plugin_helper_projects_input_frames_until_close(self) -> None:
        output = StringIO()
        input_body = "\n".join(
            [
                json.dumps(_open_frame("bidi_open")),
                json.dumps(
                    {
                        "type": "bidi_input",
                        "call_id": "call-1",
                        "frame": {"kind": "audio", "seq": 1},
                    }
                ),
                json.dumps(
                    {
                        "type": "bidi_input",
                        "call_id": "call-1",
                        "frame": {"kind": "control", "body": "close"},
                    }
                ),
                json.dumps({"type": "close", "call_id": "call-1", "reason": "client_closed"}),
            ]
        )

        def handle(
            invocation: SidecarInvocation,
            frames: Iterator[Mapping[str, object]],
        ) -> list[dict[str, object]]:
            del invocation
            return [{"kind": "ack", "input": frame["kind"]} for frame in frames]

        serve_bidi_plugin(
            handle,
            input_stream=StringIO(input_body + "\n"),
            output_stream=output,
            terminal_reason="closed",
        )

        frames = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(
            frames,
            [
                {
                    "type": "bidi_output",
                    "call_id": "call-1",
                    "frame": {"kind": "ack", "input": "audio"},
                },
                {
                    "type": "bidi_output",
                    "call_id": "call-1",
                    "frame": {"kind": "ack", "input": "control"},
                },
                {"type": "terminal", "call_id": "call-1", "reason": "closed"},
            ],
        )

    def test_bidi_plugin_helper_rejects_mismatched_input_call_id(self) -> None:
        output = StringIO()
        input_body = "\n".join(
            [
                json.dumps(_open_frame("bidi_open")),
                json.dumps(
                    {
                        "type": "bidi_input",
                        "call_id": "other-call",
                        "frame": {"kind": "audio"},
                    }
                ),
            ]
        )

        def handle(
            _invocation: SidecarInvocation,
            frames: Iterator[Mapping[str, object]],
        ) -> list[Mapping[str, object]]:
            return list(frames)

        serve_bidi_plugin(
            handle,
            input_stream=StringIO(input_body + "\n"),
            output_stream=output,
        )

        response = json.loads(output.getvalue())
        self.assertEqual(response["type"], "error")
        self.assertEqual(response["call_id"], "call-1")
        self.assertIn("does not match open call_id", response["message"])

    def test_generic_plugin_helper_rejects_unconfigured_stream_path(self) -> None:
        output = StringIO()

        serve_plugin(
            invoke_handler=lambda _invocation: {"ok": True},
            input_stream=StringIO(json.dumps(_open_frame("stream_open")) + "\n"),
            output_stream=output,
        )

        response = json.loads(output.getvalue())
        self.assertEqual(response["type"], "error")
        self.assertEqual(response["call_id"], "call-1")
        self.assertIn("stream sidecar helper is not configured", response["message"])


if __name__ == "__main__":
    unittest.main()
