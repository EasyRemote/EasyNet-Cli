import { Readable, Writable } from "node:stream";
import test from "node:test";
import assert from "node:assert/strict";

import {
  SidecarInvocation,
  SidecarProtocolError,
  serveExecPlugin,
} from "../provider/runtime/pluginexec.js";

function requestFrame() {
  return {
    type: "invoke",
    call_id: "call-1",
    invocation: {
      caller_ura: "easynet:///r/hub/user/alice",
      callee_ura: "easynet:///r/hub/device/provider",
      ability_ura: "demo.echo",
      subject_ura: "easynet:///r/hub/resource/demo",
      invocation_nonce: [1, 2, 3, 4],
      causal_context: { form: "none" },
      args: { message: "hello" },
    },
  };
}

function inputFromFrame(frame) {
  return Readable.from([`${JSON.stringify(frame)}\n`]);
}

function captureOutput() {
  const chunks = [];
  return {
    chunks,
    output: new Writable({
      write(chunk, _encoding, callback) {
        chunks.push(Buffer.from(chunk));
        callback();
      },
    }),
    json() {
      return JSON.parse(Buffer.concat(chunks).toString("utf8"));
    },
  };
}

test("SidecarInvocation projects daemon frame", () => {
  const invocation = SidecarInvocation.fromFrame(requestFrame());

  assert.equal(invocation.callID, "call-1");
  assert.equal(invocation.callerURA, "easynet:///r/hub/user/alice");
  assert.equal(invocation.calleeURA, "easynet:///r/hub/device/provider");
  assert.equal(invocation.abilityURA, "demo.echo");
  assert.equal(invocation.subjectURA, "easynet:///r/hub/resource/demo");
  assert.deepEqual(invocation.invocationNonce, [1, 2, 3, 4]);
  assert.deepEqual(invocation.causalContext, { form: "none" });
  assert.deepEqual(invocation.args, { message: "hello" });
});

test("serveExecPlugin writes result frame", async () => {
  const capture = captureOutput();

  await serveExecPlugin(
    (invocation) => ({
      ok: true,
      message: invocation.args.message,
      nonce_len: invocation.invocationNonce.length,
    }),
    {
      input: inputFromFrame(requestFrame()),
      output: capture.output,
    },
  );

  assert.deepEqual(capture.json(), {
    type: "result",
    call_id: "call-1",
    value: { ok: true, message: "hello", nonce_len: 4 },
  });
});

test("serveExecPlugin writes error frame for handler failure", async () => {
  const capture = captureOutput();

  await serveExecPlugin(
    () => {
      throw new Error("boom");
    },
    {
      input: inputFromFrame(requestFrame()),
      output: capture.output,
    },
  );

  assert.deepEqual(capture.json(), {
    type: "error",
    call_id: "call-1",
    message: "boom",
  });
});

test("SidecarInvocation rejects non-invoke frames", () => {
  const frame = requestFrame();
  frame.type = "stream_open";

  assert.throws(() => SidecarInvocation.fromFrame(frame), SidecarProtocolError);
});

test("SidecarInvocation rejects retired tuple aliases", () => {
  const frame = requestFrame();
  frame.invocation.caller = "easynet:///r/hub/user/bob";

  assert.throws(
    () => SidecarInvocation.fromFrame(frame),
    /retired/,
  );
});

test("SidecarInvocation rejects unknown invocation fields", () => {
  const frame = requestFrame();
  frame.invocation.descriptor_ref = "retired-provider-leak";

  assert.throws(
    () => SidecarInvocation.fromFrame(frame),
    /canonical invocation frame/,
  );
});

test("SidecarInvocation rejects unknown request fields", () => {
  const frame = requestFrame();
  frame.retired_mode = "json";

  assert.throws(
    () => SidecarInvocation.fromFrame(frame),
    /canonical request frame/,
  );
});

test("SidecarInvocation rejects missing canonical invocation objects", () => {
  for (const field of ["causal_context", "args"]) {
    const frame = requestFrame();
    delete frame.invocation[field];

    assert.throws(
      () => SidecarInvocation.fromFrame(frame),
      /must be an object/,
    );

    const nullFrame = requestFrame();
    nullFrame.invocation[field] = null;

    assert.throws(
      () => SidecarInvocation.fromFrame(nullFrame),
      /must be an object/,
    );
  }
});
