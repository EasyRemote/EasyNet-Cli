import assert from "node:assert/strict";
import test from "node:test";

import {
  Client,
  DEFAULT_DIRECTORY_PAGE_SIZE,
  DirectoryClient,
  ErrorCode,
  IdentityClient,
  InvocationBuilder,
  ReceiptChain,
  ReceiptClient,
  ReceiptRef,
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

const directoryBase = () => ({
  caller_ura: "easynet:///r/example/agent/alice.sdk",
  callee_ura: "easynet:///r/example/device/dev-a",
  subject_ura: "easynet:///r/example/device/dev-a",
  descriptor_version: "1.0.0",
  nonce_base64: "AQIDBAUGBwgJCgsMDQ4PEA==",
  causal_context: { form: "none" },
});

const receiptFetch = () => ({
  ...directoryBase(),
  descriptor_ref: "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
  invocation_ura: "easynet:///r/example/resource/invocation.inv-1",
});

const receiptHash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

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

test("IdentityClient delegates DescriptorRef and URA projections without local grammar", async () => {
  const seen = [];
  const identity = new IdentityClient({
    projectDescriptorRef: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "project", request });
      return JSON.stringify({
        kind: "descriptor_ref",
        valid: true,
        descriptor_ref: request.descriptor_ref,
        ability_ura: "easynet:///r/example/ability/device.dev-a.observe.health",
        descriptor_version: "1.0.0",
        profile: "directory_identity",
        components: {},
        metadata: {},
      });
    },
    buildDescriptorRef: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build", request });
      return JSON.stringify({
        kind: "descriptor_ref",
        valid: true,
        descriptor_ref: `${request.ability_ura}@${request.descriptor_version}`,
        ability_ura: request.ability_ura,
        descriptor_version: request.descriptor_version,
        profile: "directory_identity",
        components: {},
        metadata: {},
      });
    },
    ownerAbilityURA: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "owner", request });
      return JSON.stringify({
        ability_ura: "easynet:///r/example/ability/device.dev-a.observe.health",
      });
    },
    resourceURA: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "resource", request });
      return JSON.stringify({
        resource_ura: "easynet:///r/example/resource/alice.docs",
      });
    },
  });

  const ability = await identity.abilityURAFromDescriptorRef(
    "opaque-descriptor-ref-from-identity-profile",
  );
  assert.equal(ability, "easynet:///r/example/ability/device.dev-a.observe.health");

  const descriptor = await identity.ownerAbilityDescriptorRef(
    "easynet:///r/example/device/dev-a",
    "observe.health",
    "1.0.0",
  );
  assert.equal(descriptor, "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0");

  const resource = await identity.resourceURA("easynet:///r/example/agent/alice.sdk", "docs");
  assert.equal(resource, "easynet:///r/example/resource/alice.docs");
  assert.deepEqual(seen.map((item) => item.method), ["project", "owner", "build", "resource"]);

  await assert.rejects(
    () => identity.projectDescriptorRef({ descriptor_ref: " descriptor " }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("DirectoryClient delegates bounded read-model pages without fanout", async () => {
  const seen = [];
  const directory = new DirectoryClient({
    resolve: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "resolve", request });
      return JSON.stringify({ kind: "resolved_ref", profile: "directory_identity" });
    },
    listDevices: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "devices", request });
      return JSON.stringify({
        profile: "directory_identity",
        kind: "device_page",
        items: [],
        next_cursor: "",
        metadata: {},
      });
    },
    listAbilities: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "abilities", request });
      return JSON.stringify({
        profile: "directory_identity",
        kind: "ability_page",
        items: [],
        next_cursor: "",
        metadata: {},
      });
    },
  });

  await directory.resolve({ ...directoryBase(), query_name: "dev-a", ability_name: "observe.health" });
  const devices = await directory.listDevices(directoryBase());
  const abilities = await directory.listAbilities({
    ...directoryBase(),
    limit: 25,
    scope: "owner",
    owner_ura: "easynet:///r/example/device/dev-a",
  });

  assert.equal(devices.kind, "device_page");
  assert.equal(abilities.kind, "ability_page");
  assert.equal(seen[1].request.limit, DEFAULT_DIRECTORY_PAGE_SIZE);
  assert.equal(seen[2].request.limit, 25);
  assert.equal(seen[2].request.owner_ura, "easynet:///r/example/device/dev-a");

  await assert.rejects(
    () => directory.listDevices({ ...directoryBase(), limit: 501 }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  await assert.rejects(
    () => directory.resolve({ ...directoryBase() }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("DirectoryClient exposes subscription as StreamHandle seam", async () => {
  const events = [
    { kind: "directory_event", phase: "live" },
    { kind: "directory_event", phase: "terminal", terminal: true },
  ];
  const directory = new DirectoryClient({
    resolve: () => JSON.stringify({ kind: "resolved_ref" }),
    subscribeDirectory: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      assert.equal(request.stream, "directory");
      return {
        open: JSON.stringify({ stream: "directory", state: "Live" }),
        transport: {
          receive: () => JSON.stringify(events.shift()),
          close: () => {},
        },
      };
    },
  });

  const stream = await directory.subscribeDirectory(directoryBase());
  const phases = [];
  for await (const event of stream.events()) {
    phases.push(event.phase);
  }
  assert.deepEqual(phases, ["live", "terminal"]);
});

test("ReceiptClient delegates fetch, projection, and causal refs without verification claims", async () => {
  const seen = [];
  const receipt = new ReceiptClient({
    fetch: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "fetch", request });
      return JSON.stringify({
        receipt_ura: request.invocation_ura,
        invocation_id: "inv-1",
        state: "completed",
        verified: false,
        metadata: { profile: "receipt" },
      });
    },
    project: (receiptJSON) => {
      const request = JSON.parse(Buffer.from(receiptJSON).toString("utf8"));
      seen.push({ method: "project", request });
      return JSON.stringify({
        receipt_ura: request.receipt_ura,
        invocation_id: request.invocation_id,
        state: "completed",
        verified: false,
        metadata: {},
      });
    },
    verify: (receiptJSON) => {
      const request = JSON.parse(Buffer.from(receiptJSON).toString("utf8"));
      seen.push({ method: "verify", request });
      return JSON.stringify({
        verified: false,
        receipt_ura: request.receipt_ura,
        method: "provider_required",
        reason: "summary_only",
        metadata: {},
      });
    },
    causalRef: (receiptJSON) => {
      const request = JSON.parse(Buffer.from(receiptJSON).toString("utf8"));
      seen.push({ method: "causal", request });
      return JSON.stringify({
        causal_ref: `receipt:${request.receipt_ura}`,
        receipt_ura: request.receipt_ura,
        receipt_hash_hex: request.receipt_hash_hex,
        causal_context: {
          form: "receipt",
          receipt_ura: request.receipt_ura,
          receipt_hash_hex: request.receipt_hash_hex,
        },
        verified: false,
        metadata: {},
      });
    },
  });

  const fetched = await receipt.fetch(receiptFetch());
  assert.equal(fetched.verified, false);

  const ref = new ReceiptRef({
    receipt_ura: "easynet:///r/example/resource/receipt.inv-1",
    receipt_hash_hex: receiptHash,
    invocation_id: "inv-1",
  });
  const projected = await receipt.project(ref.toJSON());
  const verified = await receipt.verify(ref.toJSON());
  const causal = await ref.causalContext(receipt);

  assert.equal(projected.verified, false);
  assert.equal(verified.verified, false);
  assert.equal(causal.receipt_hash_hex, receiptHash);
  assert.deepEqual(seen.map((item) => item.method), ["fetch", "project", "verify", "causal"]);
});

test("ReceiptClient delegates carriers, history, and chain verification", async () => {
  const seen = [];
  const draftJSON = completeDraft().build().toJSONString();
  const receipt = new ReceiptClient({
    fetch: () => JSON.stringify({ verified: false, metadata: {} }),
    buildFetchInvocation: (requestJSON) => {
      seen.push({ method: "build_fetch", request: JSON.parse(Buffer.from(requestJSON).toString("utf8")) });
      return draftJSON;
    },
    buildListHistoryInvocation: (requestJSON) => {
      seen.push({ method: "build_list", request: JSON.parse(Buffer.from(requestJSON).toString("utf8")) });
      return draftJSON;
    },
    listHistory: (requestJSON) => {
      seen.push({ method: "list", request: JSON.parse(Buffer.from(requestJSON).toString("utf8")) });
      return JSON.stringify({ profile: "receipt", items: [] });
    },
    getTrace: (requestJSON) => {
      seen.push({ method: "trace", request: JSON.parse(Buffer.from(requestJSON).toString("utf8")) });
      return JSON.stringify({ profile: "receipt", trace: [] });
    },
    verifyChain: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "chain", request });
      return JSON.stringify({
        verified: false,
        continuous: true,
        method: "provider_projection",
        receipt_count: request.receipts.length,
        metadata: request.metadata ?? {},
      });
    },
  });

  const built = await receipt.buildFetchInvocation(receiptFetch());
  await receipt.buildListHistoryInvocation({ ...directoryBase(), arguments: { limit: 1 } });
  const listed = await receipt.listHistory({ ...directoryBase(), arguments: { limit: 1 } });
  const trace = await receipt.getTrace({ ...directoryBase(), arguments: { invocation_id: "inv-1" } });
  const chain = new ReceiptChain([
    { receipt_ura: "easynet:///r/example/resource/receipt.inv-1", receipt_hash_hex: receiptHash },
  ]);
  const verification = await chain.verifyContinuity(receipt, { source: "test" });

  assert.equal(built.descriptorRef, "opaque-descriptor-ref-from-identity-profile");
  assert.equal(listed.profile, "receipt");
  assert.equal(trace.profile, "receipt");
  assert.equal(verification.receipt_count, 1);
  assert.equal(seen.at(-1).request.receipts[0].receipt_hash_hex, receiptHash);
});

test("ReceiptRef rejects fabricated or malformed receipt anchors", () => {
  assert.throws(
    () => new ReceiptRef({ receipt_hash_hex: receiptHash }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  assert.throws(
    () => new ReceiptRef({ receipt_ura: "easynet:///r/example/resource/receipt.inv-1", receipt_hash_hex: "abc" }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});
