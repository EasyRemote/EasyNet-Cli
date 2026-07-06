import assert from "node:assert/strict";
import test from "node:test";

import {
  Client,
  DEFAULT_DIRECTORY_PAGE_SIZE,
  DirectoryClient,
  ErrorCode,
  HOST_STREAM_EMPTY_OUTPUT_HASH,
  HOST_STREAM_FRAME_SCHEMA,
  HOST_STREAM_HASH_ALGORITHM,
  HostBindingClient,
  HostStreamHashState,
  HealthClient,
  InvocationSignature,
  InvocationHandle,
  IdentityClient,
  InvocationBuilder,
  LocalHostBindingTransport,
  PreparedInvocation,
  PublicationClient,
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

const runtimeCoreDraft = () =>
  new InvocationBuilder()
    .withCallerURA("easynet:///r/example/agent/alice.sdk")
    .withCalleeURA("easynet:///r/example/device/dev-a")
    .withDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0")
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

const publicationResourceRef = () => ({
  resource_ura: "easynet:///r/example/resource/fs.local.pkg",
  owner_ura: "easynet:///r/example/device/dev-a",
  namespace: "fs",
  display_path: "/tmp/easynet/pkg",
  capability: "read",
  expires_unix_ms: 0,
  revision: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
});

const publicationDeploy = () => ({
  ...directoryBase(),
  resource_ref: publicationResourceRef(),
  node_id: "local",
  metadata: { request_id: "deploy-1" },
});

const publicationDraftJSON = (descriptorRef) =>
  new InvocationBuilder()
    .withCallerURA("easynet:///r/example/agent/alice.sdk")
    .withCalleeURA("easynet:///r/example/device/dev-a")
    .withDescriptorRef(descriptorRef)
    .withSubjectURA("easynet:///r/example/device/dev-a")
    .withNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==")
    .withCausalContext({ form: "none" })
    .withJSONArgs({ ok: true })
    .withContentType("application/json")
    .withMetadata({ profile: "publication" })
    .build()
    .toJSONString();

const hostBindingRequest = () => ({
  binding_id: "binding-weather-1",
  descriptor_ref: "easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
  endpoint: "/tmp/easynet-weather.sock",
  frame_schema: HOST_STREAM_FRAME_SCHEMA,
  cleanup: { mode: "unlink_socket" },
  timeout_ms: 30000,
});

const preparedInvocationJSON = (overrides = {}) => ({
  prepared_id: "prepared-example-1",
  tuple: runtimeCoreDraft().build().toJSON(),
  signing_material: {
    algorithm: "ed25519",
    canonical_bytes_base64: "ZXhhbXBsZS1jYW5vbmljYWwtYnl0ZXM=",
    args_digest_hex: "0000000000000000000000000000000000000000000000000000000000000000",
    descriptor_ref: "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
    expires_at_unix_ms: 1783000000000,
  },
  submit_ready: false,
  ...overrides,
});

const callerSignature = () =>
  new InvocationSignature({
    algorithm: "ed25519",
    signature_base64: "c2lnbmF0dXJl",
    key_id_hint: "signer-alice-key-1",
  });

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

  await assert.rejects(
    () => client.requireABI(5),
    (error) => error instanceof SDKError && error.code === ErrorCode.VERSION_MISMATCH,
  );
});

test("HealthClient decodes runtime health and diagnostics DTOs", async () => {
  const calls = [];
  const client = new HealthClient({
    runtimeHealth: () => {
      calls.push("health");
      return JSON.stringify({
        api_ready: true,
        daemon_ready: true,
        invocation_ready: true,
        directory_ready: true,
        trust_ready: true,
        runtime_ready: true,
        version: "0.1.0",
        abi_version: 4,
        mismatch: null,
        diagnostics: [],
      });
    },
    runtimeDiagnostics: () => {
      calls.push("diagnostics");
      return JSON.stringify({
        profile: "health",
        kind: "diagnostics_report",
        state: "Running",
        ready: true,
        version: "0.91.30",
        abi_version: 4,
        control_endpoint: "/tmp/easynet/control.json",
        invocation_endpoint: "/tmp/easynet/daemon.sock",
        checks: [{ name: "runtime", ready: true, message: null }],
        diagnostics: [],
      });
    },
  });

  const health = await client.runtimeHealth();
  const diagnostics = await client.diagnostics();

  assert.equal(health.apiAlive(), true);
  assert.equal(health.ready(), true);
  assert.equal(health.abiVersion, 4);
  assert.equal(health.toJSON().runtime_ready, true);
  assert.equal(diagnostics.profile, "health");
  assert.equal(diagnostics.kind, "diagnostics_report");
  assert.equal(diagnostics.checks.length, 1);
  assert.deepEqual(calls, ["health", "diagnostics"]);
});

test("HealthClient preserves API liveness separate from runtime readiness", async () => {
  const client = new HealthClient({
    runtimeHealth: () =>
      JSON.stringify({
        api_ready: true,
        daemon_ready: true,
        invocation_ready: false,
        directory_ready: true,
        trust_ready: true,
        runtime_ready: false,
        diagnostics: ["invocation endpoint unavailable"],
      }),
  });

  const health = await client.runtimeHealth();

  assert.equal(health.apiAlive(), true);
  assert.equal(health.ready(), false);
  assert.equal(health.invocationReady, false);
  assert.deepEqual(health.diagnostics, ["invocation endpoint unavailable"]);
  await assert.rejects(
    () => client.diagnostics(),
    (error) => error instanceof SDKError && error.code === ErrorCode.NOT_IMPLEMENTED,
  );
});

test("HealthClient rejects malformed payloads and wraps transport failure", async () => {
  await assert.rejects(
    () =>
      new HealthClient({
        runtimeHealth: () =>
          JSON.stringify({
            api_ready: true,
            daemon_ready: true,
            invocation_ready: true,
            directory_ready: true,
            trust_ready: true,
            runtime_ready: true,
            abi_version: true,
          }),
      }).runtimeHealth(),
    (error) =>
      error instanceof SDKError &&
      error.code === ErrorCode.INVALID_ARGUMENT &&
      error.source === "health",
  );

  const down = new Error("daemon unavailable");
  await assert.rejects(
    () =>
      new HealthClient({
        runtimeHealth: () => {
          throw down;
        },
      }).runtimeHealth(),
    (error) =>
      error instanceof SDKError &&
      error.code === ErrorCode.ROUTE_UNAVAILABLE &&
      error.cause === down,
  );
});

test("HealthClient closes transport and rejects closed use", async () => {
  const calls = [];
  const client = new HealthClient({
    runtimeHealth: () =>
      JSON.stringify({
        api_ready: true,
        daemon_ready: true,
        invocation_ready: true,
        directory_ready: true,
        trust_ready: true,
        runtime_ready: true,
        diagnostics: [],
      }),
    close: () => {
      calls.push("close");
    },
  });

  await client.close();
  await client.close();
  assert.deepEqual(calls, ["close"]);
  await assert.rejects(
    () => client.runtimeHealth(),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("InvocationBuilder validates tuple completeness without descriptor grammar", () => {
  const builder = completeDraft();
  const inspected = builder.inspect();
  assert.equal(inspected.descriptorRef, "opaque-descriptor-ref-from-identity-profile");

  const draft = builder.build();
  assert.equal(draft.descriptorRef, "opaque-descriptor-ref-from-identity-profile");
  assert.equal(draft.toJSON().descriptor_ref, "opaque-descriptor-ref-from-identity-profile");
  assert.throws(
    () => builder.inspect(),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_HANDLE,
  );

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

test("RuntimeClient.prepare returns daemon-provided canonical signing material", async () => {
  const seen = [];
  const runtime = new RuntimeClient({
    invoke: () => JSON.stringify({ ok: true }),
    prepare: (draftJSON, optionsJSON) => {
      seen.push({
        draft: JSON.parse(Buffer.from(draftJSON).toString("utf8")),
        options: JSON.parse(Buffer.from(optionsJSON).toString("utf8")),
      });
      return JSON.stringify(preparedInvocationJSON());
    },
  });

  const prepared = await runtime.prepare(runtimeCoreDraft().build(), { deadline_unix_ms: 1783000000000 });

  assert.equal(prepared instanceof PreparedInvocation, true);
  assert.equal(prepared.submitReady(), false);
  assert.equal(prepared.preparedId, "prepared-example-1");
  assert.equal(prepared.signingMaterial.algorithm, "ed25519");
  assert.equal(prepared.signingMaterial.canonicalBytesBase64, "ZXhhbXBsZS1jYW5vbmljYWwtYnl0ZXM=");
  assert.equal(
    prepared.signingMaterial.descriptorRef,
    "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
  );
  assert.equal(seen[0].draft.descriptor_ref, prepared.signingMaterial.descriptorRef);
  assert.deepEqual(seen[0].options, { deadline_unix_ms: 1783000000000 });
});

test("PreparedInvocation enforces non-submit-ready canonical material boundaries", () => {
  assert.throws(
    () => new PreparedInvocation(preparedInvocationJSON({ submit_ready: true })),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  assert.throws(
    () => new PreparedInvocation(preparedInvocationJSON({ submit_ready: "false" })),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );

  assert.throws(
    () =>
      new PreparedInvocation(
        preparedInvocationJSON({
          signing_material: {
            ...preparedInvocationJSON().signing_material,
            descriptor_ref: "easynet:///r/example/ability/other@1.0.0",
          },
        }),
      ),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );

  assert.throws(
    () =>
      new PreparedInvocation(
        preparedInvocationJSON({
          canonical_hash_hex: "0000000000000000000000000000000000000000000000000000000000000000",
        }),
      ),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("PreparedInvocation signs with caller signature without rewriting daemon material", () => {
  const prepared = new PreparedInvocation(preparedInvocationJSON());
  const signed = prepared.signWithCallerSignature(callerSignature());
  const encoded = signed.toJSON();

  assert.equal(signed.submitReady(), true);
  assert.equal(encoded.signer_id, "signer-alice-key-1");
  assert.equal(encoded.signature.algorithm, "ed25519");
  assert.equal(encoded.signature.signature_base64, "c2lnbmF0dXJl");
  assert.equal(encoded.prepared.canonical_bytes_base64, prepared.signingMaterial.canonicalBytesBase64);
  assert.equal(encoded.prepared.descriptor_ref, prepared.descriptorRef);
});

test("RuntimeClient.submitSigned rejects prepared invocations before transport", async () => {
  const runtime = new RuntimeClient({
    invoke: () => JSON.stringify({ ok: true }),
    submitSigned: () => {
      throw new Error("transport must not receive prepared invocation");
    },
  });

  await assert.rejects(
    () => runtime.submitSigned(new PreparedInvocation(preparedInvocationJSON())),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("RuntimeClient submits signed envelopes and observes invocation handles", async () => {
  const seen = [];
  const runtime = new RuntimeClient({
    invoke: () => JSON.stringify({ ok: true }),
    prepare: (draftJSON) => {
      seen.push(["prepare", JSON.parse(Buffer.from(draftJSON).toString("utf8"))]);
      return JSON.stringify(preparedInvocationJSON());
    },
    submitSigned: (signedJSON) => {
      seen.push(["submit", JSON.parse(Buffer.from(signedJSON).toString("utf8"))]);
      return JSON.stringify({
        handle_id: 7,
        state: "Submitted",
        terminal: false,
        events: [{ sequence: 1, kind: "submitted", state: "Submitted", terminal: false }],
        result: null,
      });
    },
    awaitHandle: (handleId) => {
      seen.push(["await", handleId]);
      return JSON.stringify({
        ok: true,
        terminal_state: "Completed",
        output_json: {},
        receipt: null,
        error: null,
      });
    },
    cancelHandle: (handleId, reason) => {
      seen.push(["cancel", handleId, reason]);
      return JSON.stringify({
        handle_id: 7,
        cancelled: false,
        state: "Completed",
        terminal: true,
      });
    },
    handleEvents: (handleId) => {
      seen.push(["events", handleId]);
      return JSON.stringify({
        handle_id: 7,
        state: "Completed",
        terminal: true,
        events: [
          {
            sequence: 2,
            kind: "terminal",
            state: "Completed",
            terminal: true,
            result: { ok: true },
          },
        ],
        result: { ok: true },
      });
    },
    freeHandle: (handleId) => {
      seen.push(["free", handleId]);
    },
  });

  const prepared = await runtime.prepare(runtimeCoreDraft().build());
  const signed = prepared.signWithCallerSignature(callerSignature());
  const handle = await runtime.submitSigned(signed);
  const result = await handle.awaitResult();
  const cancel = await handle.cancel("after terminal");
  const refreshed = await handle.refreshEvents();
  await handle.close();

  assert.equal(handle.handleId, 7);
  assert.equal(handle.state, "Submitted");
  assert.equal(handle.events[0].sequence, 1);
  assert.equal(result.terminal_state, "Completed");
  assert.equal(cancel.state, "Completed");
  assert.equal(cancel.cancelled, false);
  assert.equal(refreshed.terminal, true);
  assert.equal(refreshed.events.length, 1);
  assert.equal(refreshed.events[0].terminal, true);
  assert.equal(seen[0][0], "prepare");
  assert.equal(seen[1][0], "submit");
  assert.equal(seen[1][1].signature.signature_base64, "c2lnbmF0dXJl");
  assert.equal(seen[1][1].prepared.canonical_bytes_base64, "ZXhhbXBsZS1jYW5vbmljYWwtYnl0ZXM=");
  assert.deepEqual(seen.slice(2), [
    ["await", 7],
    ["cancel", 7, "after terminal"],
    ["events", 7],
    ["free", 7],
  ]);
});

test("InvocationHandle rejects legacy aliases and terminal drift", () => {
  assert.throws(
    () =>
      new InvocationHandle({
        handleId: 7,
        state: "Submitted",
        terminal: false,
      }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  assert.throws(
    () =>
      new InvocationHandle({
        handle_id: 7,
        state: "Completed",
        terminal: true,
        events: [
          { sequence: 1, kind: "terminal", state: "Completed", terminal: true },
          { sequence: 2, kind: "cancelled", state: "Cancelled", terminal: true },
        ],
      }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  assert.throws(
    () =>
      new InvocationHandle({
        handle_id: 7,
        state: "Submitted",
        terminal: false,
        events: [{ sequence: 1, kind: "terminal", state: "Completed", terminal: true }],
      }),
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
    listAgents: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "agents", request });
      return JSON.stringify({
        profile: "directory_identity",
        kind: "agent_page",
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
  const agents = await directory.listAgents({ ...directoryBase(), limit: 10 });
  const abilities = await directory.listAbilities({
    ...directoryBase(),
    limit: 25,
    scope: "owner",
    owner_ura: "easynet:///r/example/device/dev-a",
  });

  assert.equal(devices.kind, "device_page");
  assert.equal(agents.kind, "agent_page");
  assert.equal(abilities.kind, "ability_page");
  assert.equal(seen[1].request.limit, DEFAULT_DIRECTORY_PAGE_SIZE);
  assert.equal(seen[2].request.limit, 10);
  assert.equal(seen[3].request.limit, 25);
  assert.equal(seen[3].request.owner_ura, "easynet:///r/example/device/dev-a");

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
    buildDirectorySubscriptionInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      assert.equal(request.stream, "directory");
      return completeDraft().build().toJSONString();
    },
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

  const carrier = await directory.buildDirectorySubscriptionInvocation(directoryBase());
  assert.equal(carrier.descriptor_ref, "opaque-descriptor-ref-from-identity-profile");

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

test("PublicationClient delegates resource, package, deploy, and unpublish carriers", async () => {
  const seen = [];
  const publication = new PublicationClient({
    buildResourceRef: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "resource", request });
      return JSON.stringify(publicationResourceRef());
    },
    validatePackage: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "validate", request });
      return JSON.stringify({
        profile: "publication",
        kind: "package_validation",
        valid: true,
        package_path: request.package_path,
        manifest_path: `${request.package_path}/ability.json`,
        manifest_hash: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        manifest: {
          name: "weather",
          namespace: "er",
          wire_key: "er.weather",
          descriptor_version: "1.0.0",
          description: "",
          exec_kind: "host_stream",
          timeout_seconds: null,
          input_schema: {},
          output_schema: null,
        },
        errors: [],
        metadata: {},
      });
    },
    buildDeployInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "deploy_carrier", request });
      return publicationDraftJSON("easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0");
    },
    buildUnpublishInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "unpublish_carrier", request });
      return publicationDraftJSON("easynet:///r/example/ability/device.dev-a.ability.unpublish@1.0.0");
    },
  });

  const ref = await publication.buildLocalResourceRef({
    path: "/tmp/easynet/pkg",
    capability: "read",
  });
  const validation = await publication.validatePackage({ package_path: "/tmp/easynet/pkg" });
  const deploy = await publication.buildDeployInvocation(publicationDeploy());
  const unpublish = await publication.buildUnpublishInvocation({
    ...directoryBase(),
    ability_ura: "easynet:///r/example/ability/device.dev-a.er.weather",
    metadata: { request_id: "unpublish-1" },
  });

  assert.equal(ref.resource_ura, "easynet:///r/example/resource/fs.local.pkg");
  assert.equal(validation.valid, true);
  assert.equal(deploy.descriptorRef, "easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0");
  assert.equal(unpublish.descriptorRef, "easynet:///r/example/ability/device.dev-a.ability.unpublish@1.0.0");
  assert.deepEqual(seen.map((item) => item.method), [
    "resource",
    "validate",
    "deploy_carrier",
    "unpublish_carrier",
  ]);
  assert.equal(seen[2].request.resource_ref.resource_ura, ref.resource_ura);
});

test("PublicationClient delegates read models and lifecycle projections", async () => {
  const seen = [];
  const publication = new PublicationClient({
    buildResourceRef: () => JSON.stringify(publicationResourceRef()),
    listAbilities: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "list", request });
      return JSON.stringify({
        profile: "publication",
        kind: "published_ability_page",
        item_kind: "published_ability",
        items: [],
        next_cursor: null,
        limit: request.limit,
        source: "daemon_read_model",
        metadata: {},
      });
    },
    showAbility: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "show", request });
      return JSON.stringify({
        descriptor: { descriptor_ref: request.descriptor_ref },
        implementation: {},
        metadata: {},
      });
    },
    installPlugin: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "install", request });
      return JSON.stringify({
        profile: "publication",
        kind: "plugin_install_result",
        source: request.source,
        install_id: "install-1",
        status: "installed",
        metadata: {},
      });
    },
    enableAbilityImpl: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "enable", request });
      return JSON.stringify({
        profile: "publication",
        kind: "ability_impl_enabled",
        status: "enabled",
        metadata: {},
      });
    },
    disableAbilityImpl: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "disable", request });
      return JSON.stringify({
        profile: "publication",
        kind: "ability_impl_disabled",
        status: "disabled",
        metadata: {},
      });
    },
    unpublishAbility: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "unpublish", request });
      return JSON.stringify({
        profile: "publication",
        kind: "ability_unpublished",
        status: "unpublished",
        metadata: {},
      });
    },
  });

  const page = await publication.listAbilities({ ...directoryBase() });
  const ability = await publication.showAbility({
    descriptor_ref: "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
  });
  const installed = await publication.installPlugin({ source: "/tmp/easynet/plugin" });
  const enabled = await publication.enableAbilityImpl({
    impl_id: "impl-1",
    ability_ura: "easynet:///r/example/ability/device.dev-a.er.weather",
  });
  const disabled = await publication.disableAbilityImpl({
    impl_id: "impl-1",
    ability_ura: "easynet:///r/example/ability/device.dev-a.er.weather",
  });
  const unpublished = await publication.unpublishAbility({
    ...directoryBase(),
    ability_ura: "easynet:///r/example/ability/device.dev-a.er.weather",
  });

  assert.equal(page.limit, 50);
  assert.equal(ability.descriptor.descriptor_ref, "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0");
  assert.equal(installed.status, "installed");
  assert.equal(enabled.status, "enabled");
  assert.equal(disabled.status, "disabled");
  assert.equal(unpublished.status, "unpublished");
  assert.deepEqual(seen.map((item) => item.method), [
    "list",
    "show",
    "install",
    "enable",
    "disable",
    "unpublish",
  ]);
});

test("PublicationClient rejects incomplete carriers and local resource fabrication", async () => {
  const publication = new PublicationClient({
    buildResourceRef: () => JSON.stringify(publicationResourceRef()),
  });

  await assert.rejects(
    () => publication.buildLocalResourceRef({ path: "relative/pkg", capability: "read" }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  await assert.rejects(
    () =>
      publication.buildDeployInvocation({
        ...publicationDeploy(),
        resource_ref: { ...publicationResourceRef(), namespace: "system" },
      }),
    (error) => error instanceof SDKError && error.source === "publication",
  );
  await assert.rejects(
    () => publication.buildUnpublishInvocation({ ...directoryBase() }),
    (error) => error instanceof SDKError && error.source === "publication",
  );

  await publication.close();
  await publication.close();
  await assert.rejects(
    () => publication.buildLocalResourceRef({ path: "/tmp/easynet/pkg", capability: "read" }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("HostBindingClient local transport builds binding and codec frames", async () => {
  const calls = [];
  const transport = new LocalHostBindingTransport((descriptorRef) => {
    calls.push(`canonical:${descriptorRef}`);
    return descriptorRef;
  });
  const client = new HostBindingClient(transport);

  const binding = await client.buildHostStreamBinding(hostBindingRequest());
  const request = await client.decodeRequest({
    request: {
      fn: "weather.stream",
      args: { city: "Singapore" },
      call_id: "call-weather-1",
      caller: "easynet:///r/example/user/alice",
    },
  });
  const item = await client.encodeItem(0, { token: "hello" });
  const error = await client.encodeError(
    new SDKError({
      code: ErrorCode.GENERIC,
      stage: "host_binding",
      retry: "never",
      message: "boom",
      details: {},
    }),
  );
  const plainError = await client.encodeError(new Error("plain boom"));
  const terminal = await client.encodeTerminal({
    output_hash: "sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15",
    frames: 1,
  });

  assert.equal(binding.binding_id, "binding-weather-1");
  assert.equal(binding.lifecycle.frame_contract_owner, "daemon_sdk");
  assert.equal(binding.metadata.hash_algorithm, HOST_STREAM_HASH_ALGORITHM);
  assert.equal(request.function, "weather.stream");
  assert.deepEqual(request.args, { city: "Singapore" });
  assert.equal(item.frame_type, "item");
  assert.equal(item.seq, 0);
  assert.equal(item.output_hash, null);
  assert.equal(error.frame_type, "error");
  assert.equal(error.error.code, ErrorCode.GENERIC);
  assert.equal(plainError.error.code, ErrorCode.GENERIC);
  assert.equal(plainError.error.message, "plain boom");
  assert.equal(terminal.frame_type, "terminal");
  assert.equal(terminal.output_hash, terminal.terminal.output_hash);
  assert.deepEqual(calls, ["canonical:easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0"]);
});

test("HostBindingClient folds output hash and rejects corrupted state", async () => {
  const client = new HostBindingClient(
    new LocalHostBindingTransport((descriptorRef) => descriptorRef),
  );
  const initial = HostStreamHashState.initial();
  assert.equal(initial.outputHash, HOST_STREAM_EMPTY_OUTPUT_HASH);

  const folded = await client.foldOutputHash(initial, 0, { token: "hello" });
  assert.equal(folded.outputHash, "sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15");
  assert.equal(folded.canonicalJSON, "{\"token\":\"hello\"}");
  assert.equal(folded.lastSeq, 0);

  await assert.rejects(
    () => client.foldOutputHash(initial, 2, { token: "skip" }),
    (error) => error instanceof SDKError && error.source === "host_binding",
  );
  assert.throws(
    () =>
      new HostStreamHashState({
        algorithm: HOST_STREAM_HASH_ALGORITHM,
        output_hash: HOST_STREAM_EMPTY_OUTPUT_HASH,
        frames: 0,
        last_seq: 0,
      }),
    (error) => error instanceof SDKError && error.source === "host_binding",
  );
  assert.throws(
    () =>
      new HostStreamHashState({
        algorithm: HOST_STREAM_HASH_ALGORITHM,
        output_hash: folded.toJSON().output_hash,
        frames: 3,
        last_seq: 0,
      }),
    (error) => error instanceof SDKError && error.source === "host_binding",
  );
});

test("HostBinding lifecycle provider is explicit and cleanup is idempotent", async () => {
  const providerCalls = [];
  const provider = {
    checkReadiness(binding) {
      providerCalls.push(`readiness:${binding.binding_id}`);
      return { state: "ready", checked: true, endpoint_ready: true };
    },
    cleanup(binding) {
      providerCalls.push(`cleanup:${binding.binding_id}`);
      return { mode: "unlink_socket", cleaned: true };
    },
  };
  const client = new HostBindingClient(
    new LocalHostBindingTransport((descriptorRef) => descriptorRef),
    provider,
  );
  const binding = await client.buildHostStreamBinding(hostBindingRequest());
  const lifecycle = client.openLifecycle(binding);
  const readiness = await lifecycle.checkReadiness();
  const cleanup = await lifecycle.cleanup();
  const cleanupAgain = await lifecycle.cleanup();

  assert.equal(readiness.state, "ready");
  assert.equal(cleanup.mode, "unlink_socket");
  assert.equal(cleanupAgain, cleanup);
  assert.equal(lifecycle.state, "cleaned");
  assert.deepEqual(providerCalls, ["readiness:binding-weather-1", "cleanup:binding-weather-1"]);
  lifecycle.close();
  assert.equal(lifecycle.state, "closed");
});

test("HostBinding rejects descriptor, endpoint, schema, and hash drift", async () => {
  const client = new HostBindingClient(new LocalHostBindingTransport());
  await assert.rejects(
    () => client.buildHostStreamBinding(hostBindingRequest()),
    (error) => error instanceof SDKError && error.source === "host_binding",
  );
  const canonicalClient = new HostBindingClient(
    new LocalHostBindingTransport((descriptorRef) => descriptorRef),
  );
  await assert.rejects(
    () => canonicalClient.buildHostStreamBinding({ ...hostBindingRequest(), endpoint: "relative.sock" }),
    (error) => error instanceof SDKError && error.source === "host_binding",
  );
  await assert.rejects(
    () => canonicalClient.buildHostStreamBinding({ ...hostBindingRequest(), frame_schema: "drift.schema.json" }),
    (error) => error instanceof SDKError && error.source === "host_binding",
  );
  await assert.rejects(
    () =>
      canonicalClient.foldOutputHash(
        {
          algorithm: HOST_STREAM_HASH_ALGORITHM,
          output_hash: "sha256:ABC",
          frames: 0,
          last_seq: null,
        },
        0,
        { token: "hello" },
      ),
    (error) => error instanceof SDKError && error.source === "host_binding",
  );
});
