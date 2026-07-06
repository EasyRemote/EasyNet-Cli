import assert from "node:assert/strict";
import test from "node:test";

import {
  Client,
  ErrorCode,
  InvocationBuilder,
  RuntimeClient,
  SDKError,
} from "../index.js";

const completeDraft = () =>
  new InvocationBuilder()
    .withCallerURA("easynet:///r/example/agent/alice.sdk")
    .withCalleeURA("easynet:///r/example/device/dev-a")
    .withDescriptorRef("opaque-descriptor-ref-from-identity-profile")
    .withSubjectURA("easynet:///r/example/device/dev-a")
    .withNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==")
    .withCausalContext({ form: "none" })
    .withJSONArgs({})
    .withContentType("application/json");

test("feature discovery decodes canonical Runtime Core facts", async () => {
  const client = new Client({
    featureDiscovery: () =>
      JSON.stringify({
        abi_version: 4,
        sdk_version: "0.91.30",
        profiles: { runtime_core: "seam" },
        symbols: { runtime_health: true },
        axon_pb: false,
      }),
  });

  const features = await client.requireABI(4);
  assert.equal(features.abiVersion, 4);
  assert.equal(features.version().sdkVersion, "0.91.30");
  assert.equal(features.symbols.runtime_health, true);
});

test("InvocationBuilder validates tuple completeness without descriptor grammar", () => {
  const draft = completeDraft().build();
  assert.equal(draft.descriptorRef, "opaque-descriptor-ref-from-identity-profile");
  assert.equal(draft.toJSON().descriptor_ref, "opaque-descriptor-ref-from-identity-profile");

  assert.throws(
    () =>
      new InvocationBuilder()
        .withCallerURA("easynet:///r/example/agent/alice.sdk")
        .withCalleeURA("easynet:///r/example/device/dev-a")
        .withDescriptorRef("descriptor")
        .withSubjectURA("easynet:///r/example/device/dev-a")
        .withNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==")
        .withCausalContext({ form: "none" })
        .withContentType("application/json")
        .build(),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("RuntimeClient delegates through injected transport and rejects closed use", async () => {
  const seen = [];
  const runtime = new RuntimeClient({
    invoke: (draftJSON) => {
      seen.push(JSON.parse(Buffer.from(draftJSON).toString("utf8")));
      return JSON.stringify({ ok: true, terminal_state: "Completed" });
    },
    close: () => {
      seen.push({ closed: true });
    },
  });

  const result = await runtime.invoke(completeDraft().build());
  assert.equal(result.ok, true);
  assert.equal(seen[0].caller_ura, "easynet:///r/example/agent/alice.sdk");

  await runtime.close();
  await runtime.close();
  assert.deepEqual(seen.at(-1), { closed: true });
  await assert.rejects(
    () => runtime.invoke(completeDraft().build()),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("typed daemon error JSON decodes canonical schema values", () => {
  const error = SDKError.fromJSON(
    JSON.stringify({
      code: "DAEMON_OFFLINE",
      stage: "transport",
      message: "daemon offline",
      retry: "safe",
      details: { profile: "runtime_core" },
    }),
  );

  assert.equal(error.code, ErrorCode.DAEMON_OFFLINE);
  assert.equal(error.retryable, true);
  assert.equal(error.details.profile, "runtime_core");
});

test("typed daemon error JSON rejects legacy code aliases", () => {
  for (const code of ["InvalidArgument", "DaemonDown", "DAEMON_DOWN", "VersionIncompatible"]) {
    assert.throws(
      () =>
        SDKError.fromJSON(
          JSON.stringify({
            code,
            stage: "transport",
            message: "legacy code",
            retry: "never",
            details: {},
          }),
        ),
      (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
    );
  }
});

test("StreamHandle exposes async iteration with terminal close", async () => {
  const closed = [];
  const events = [
    { frame_type: "data", value: 1 },
    { frame_type: "terminal", terminal: true, state: "Completed" },
  ];
  const runtime = new RuntimeClient({
    invoke: () => JSON.stringify({ ok: true }),
    openStream: () => ({
      open: JSON.stringify({ stream_id: "stream-1", state: "Open" }),
      transport: {
        receive: () => JSON.stringify(events.shift()),
        close: () => {
          closed.push("stream-1");
        },
      },
    }),
  });

  const stream = await runtime.invokeStream(completeDraft().build());
  const seen = [];
  for await (const event of stream) {
    seen.push(event);
  }

  assert.deepEqual(seen.map((event) => event.frame_type), ["data", "terminal"]);
  assert.equal(stream.terminal, true);
  assert.deepEqual(closed, ["stream-1"]);
  await assert.rejects(
    () => stream.receive(),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("StreamHandle AbortSignal cancellation calls transport cancel", async () => {
  const cancelled = [];
  const controller = new AbortController();
  const runtime = new RuntimeClient({
    invoke: () => JSON.stringify({ ok: true }),
    openStream: () => ({
      open: JSON.stringify({ stream_id: "stream-2", state: "Open" }),
      transport: {
        receive: () => new Promise(() => {}),
        cancel: (reason) => {
          cancelled.push(reason);
        },
      },
    }),
  });

  const stream = await runtime.invokeStream(completeDraft().build());
  const pending = stream.receive({ signal: controller.signal, cancelReason: "operator cancelled" });
  controller.abort("ignored by explicit reason");

  await assert.rejects(
    pending,
    (error) => error instanceof SDKError && error.code === ErrorCode.CANCELLED,
  );
  assert.deepEqual(cancelled, ["operator cancelled"]);
  assert.equal(stream.closed, true);
});

test("BidiSession exposes async iteration and AbortSignal cancellation", async () => {
  const closed = [];
  const cancelled = [];
  const frames = [
    { frame_type: "data", payload: { ok: true } },
    { frame_type: "done", terminal: true, state: "Closed" },
  ];
  const runtime = new RuntimeClient({
    invoke: () => JSON.stringify({ ok: true }),
    openBidi: () => ({
      open: JSON.stringify({ session_id: "bidi-1", state: "Open" }),
      transport: {
        send: () => {},
        receive: () => (frames.length > 0 ? JSON.stringify(frames.shift()) : new Promise(() => {})),
        close: () => {
          closed.push("bidi-1");
        },
        cancel: (reason) => {
          cancelled.push(reason);
        },
      },
    }),
  });

  const bidi = await runtime.openBidi(completeDraft().build());
  await bidi.send({ frame_type: "data", payload: { hello: true } });
  const seen = [];
  for await (const frame of bidi.frames()) {
    seen.push(frame);
  }
  assert.deepEqual(seen.map((frame) => frame.frame_type), ["data", "done"]);
  assert.deepEqual(closed, ["bidi-1"]);

  const aborted = await runtime.openBidi(completeDraft().build());
  const controller = new AbortController();
  const pending = aborted.receive({ signal: controller.signal });
  controller.abort("stop bidi");
  await assert.rejects(
    pending,
    (error) => error instanceof SDKError && error.code === ErrorCode.CANCELLED,
  );
  assert.deepEqual(cancelled, ["stop bidi"]);
});
