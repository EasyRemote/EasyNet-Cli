import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import * as sdk from "../index.js";

const caller = "easynet:///r/example/agent/alice.sdk";
const callee = "easynet:///r/example/device/dev-a";
const descriptor = "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0";
const nonce = "AQIDBAUGBwgJCgsMDQ4PEA==";

const expectedExports = [
  "AUTHORITY_PROFILE",
  "AuthorityClient",
  "AuthorityMetadata",
  "BidiSession",
  "Client",
  "DELEGATION_METADATA_KEY",
  "DelegationProof",
  "DelegationRequest",
  "DiagnosticCheck",
  "DiagnosticsReport",
  "ErrorClass",
  "ErrorCode",
  "HEALTH_PROFILE",
  "HealthClient",
  "InvocationBuilder",
  "InvocationCancel",
  "InvocationDraft",
  "InvocationHandle",
  "InvocationHandleEvent",
  "InvocationSignature",
  "MAX_BIDI_BUFFERED_FRAMES",
  "MAX_STREAM_BUFFERED_EVENTS",
  "PreparedInvocation",
  "RetryHint",
  "RuntimeClient",
  "RuntimeHealth",
  "SDKError",
  "SESSION_AUTHORITY_METADATA_KEY",
  "SessionAuthority",
  "SessionAuthorityRequest",
  "SignedInvocation",
  "SignerPolicy",
  "SigningMaterial",
  "StreamHandle",
  "profileErrorDetails",
  "profileSourceRef",
];

const productSymbols = [
  "AdminClient",
  "CompanionClient",
  "CompatibilityClient",
  "DirectoryClient",
  "IdentityClient",
  "EventClient",
  "HostBindingClient",
  "MissionClient",
  "PublicationClient",
  "ReceiptClient",
  "SurfaceClient",
  "WrapperClient",
];

const completeDraft = () =>
  new sdk.InvocationBuilder()
    .withCallerURA(caller)
    .withCalleeURA(callee)
    .withDescriptorRef(descriptor)
    .withSubjectURA(callee)
    .withNonceBase64(nonce)
    .withCausalContext({ form: "none" })
    .withJSONArgs({ probe: true })
    .withContentType("application/json")
    .withMetadata({ trace_id: "trace-1" })
    .build();

const authorityValue = (payload) =>
  Buffer.from(
    JSON.stringify({
      payload,
      signature: Buffer.from("signature").toString("base64"),
    }),
  ).toString("base64");

const delegationValue = () =>
  authorityValue({
    issuer_ura: "easynet:///r/example/user/alice",
    subject_ura: "easynet:///r/example/user/alice",
    caller_ura: caller,
    audience: callee,
    scopes: ["invoke"],
    issued_at_ms: 10,
    expires_at_ms: 20,
  });

const sessionValue = () =>
  authorityValue({
    issuer_ura: caller,
    session_id: "session-1",
    session_owner_user_id: "alice",
    creator_principal_id: caller,
    callee_ura: callee,
    subject_ura: "easynet:///r/example/user/alice",
    audience: callee,
    scopes: ["invoke"],
    allowed_actions: ["invoke"],
    allowed_followup_abilities: ["observe.health"],
    issued_at_ms: 10,
    expires_at_ms: 20,
  });

const preparedJSON = (draft) =>
  JSON.stringify({
    prepared_id: "prepared-1",
    request_id: "request-1",
    tuple: draft,
    signing_material: {
      algorithm: "ed25519",
      canonical_bytes_base64: Buffer.from("canonical").toString("base64"),
      args_digest_hex: "a".repeat(64),
      descriptor_ref: descriptor,
      nonce_base64: nonce,
      signed_fields: ["caller_ura", "callee_ura", "descriptor_ref", "subject_ura"],
      expires_at_unix_ms: 4102444800000,
    },
    descriptor_ref: descriptor,
    descriptor_hash_hex: "",
    schema_hash_hex: "",
    canonical_hash_hex: "",
    expires_at_unix_ms: 4102444800000,
    submit_ready: false,
  });

test("runtime package exports exactly the generic public surface", async () => {
  assert.deepEqual(Object.keys(sdk).sort(), expectedExports);

  const declarations = await readFile(new URL("../index.d.ts", import.meta.url), "utf8");
  for (const product of productSymbols) {
    assert.equal(declarations.includes(product), false, `${product} leaked through index.d.ts`);
  }
  for (const exported of expectedExports.filter((name) => /^[A-Z]/.test(name))) {
    assert.match(declarations, new RegExp(`\\b${exported}\\b`));
  }
});

test("feature discovery and client lifecycle are explicit", async () => {
  let closed = 0;
  const client = new sdk.Client({
    featureDiscovery: () =>
      JSON.stringify({
        abi_version: 5,
        sdk_version: "0.0.0-seam",
        profiles: { runtime_core: "seam", health: "seam", authority: "seam" },
        symbols: { runtime_prepare: true, runtime_submit_signed: true },
        axon_pb: false,
      }),
    close: () => {
      closed += 1;
    },
  });

  const features = await client.requireABI(5);
  assert.equal(features.profiles.runtime_core, "seam");
  assert.equal(features.symbols.runtime_prepare, true);
  await assert.rejects(
    () => client.requireABI(4),
    (error) => error instanceof sdk.SDKError && error.code === sdk.ErrorCode.VERSION_MISMATCH,
  );
  await client.close();
  await client.close();
  assert.equal(closed, 1);
  await assert.rejects(
    () => client.featureDiscovery(),
    (error) => error instanceof sdk.SDKError && error.code === sdk.ErrorCode.INVALID_ARGUMENT,
  );
});

test("health keeps API liveness separate from runtime readiness", async () => {
  const health = new sdk.HealthClient({
    runtimeHealth: () =>
      JSON.stringify({
        api_ready: true,
        invocation_ready: false,
        directory_ready: false,
        trust_ready: true,
        runtime_ready: false,
        version: "0.0.0-seam",
        abi_version: 5,
        mismatch: null,
        diagnostics: ["runtime warming"],
      }),
    runtimeDiagnostics: () =>
      JSON.stringify({
        profile: "health",
        kind: "diagnostics_report",
        state: "Running",
        ready: false,
        version: "0.0.0-seam",
        abi_version: 5,
        control_endpoint: "/tmp/easynet-control.sock",
        invocation_endpoint: "/tmp/easynet-daemon.sock",
        checks: [{ name: "runtime", ready: false, message: "warming" }],
        diagnostics: ["runtime warming"],
      }),
  });

  const state = await health.runtimeHealth();
  assert.equal(state.apiAlive(), true);
  assert.equal(state.ready(), false);
  assert.equal(state.abiVersion, 5);
  const diagnostics = await health.diagnostics();
  assert.equal(diagnostics.checks.length, 1);
  assert.equal(diagnostics.ready, false);
  await health.close();
});

test("prepare, caller-sign, submit, and handle lifecycle preserve generic invocation facts", async () => {
  const calls = [];
  const runtime = new sdk.RuntimeClient({
    invoke: (draftJSON) => {
      const draft = JSON.parse(Buffer.from(draftJSON).toString("utf8"));
      calls.push(["invoke", draft]);
      return JSON.stringify({
        ok: true,
        terminal_state: "Completed",
        output: { ok: true },
        terminal_receipt: { receipt_ref: "opaque-receipt-ref", receipt_hash: "opaque-hash" },
      });
    },
    prepare: (draftJSON, optionsJSON) => {
      const draft = JSON.parse(Buffer.from(draftJSON).toString("utf8"));
      const options = JSON.parse(Buffer.from(optionsJSON).toString("utf8"));
      calls.push(["prepare", draft, options]);
      return preparedJSON(draft);
    },
    submitSigned: (signedJSON) => {
      const signed = JSON.parse(Buffer.from(signedJSON).toString("utf8"));
      calls.push(["submit", signed]);
      return JSON.stringify({ handle_id: 7, state: "Running", terminal: false, events: [], result: null });
    },
    awaitHandle: (control) => {
      calls.push(["await", control._adapterHandleId()]);
      return JSON.stringify({
        ok: true,
        terminal_state: "Completed",
        terminal_receipt: { receipt_ref: "opaque-receipt-ref" },
      });
    },
    cancelHandle: (control, reason) => {
      const handleId = control._adapterHandleId();
      calls.push(["cancel", handleId, reason]);
      return JSON.stringify({ handle_id: handleId, request_accepted: true, deduplicated: false, cancelled: true, state: "Cancelled", terminal: true });
    },
    handleEvents: (control) => {
      const handleId = control._adapterHandleId();
      calls.push(["events", handleId]);
      return JSON.stringify({
        handle_id: handleId,
        state: "Completed",
        terminal: true,
        events: [{ sequence: 1, kind: "completed", state: "Completed", terminal: true }],
        result: { ok: true },
      });
    },
    freeHandle: (control) => {
      calls.push(["free", control._adapterHandleId()]);
    },
  });

  const draft = completeDraft();
  const invoked = await runtime.invoke(draft);
  assert.equal(invoked.terminalReceipt.receipt_ref, "opaque-receipt-ref");
  assert.equal(Object.hasOwn(invoked, "receipt"), false);
  assert.equal(Object.hasOwn(invoked, "terminal_receipt"), false);
  const prepared = await runtime.prepare(draft, { deadline_ms: 1000 });
  assert.equal(prepared.submitReady(), false);
  assert.equal(prepared.tuple.callerURA, caller);
  assert.equal(prepared.tuple.descriptorRef, descriptor);
  const signed = prepared.signWithCallerSignature({
    algorithm: "ed25519",
    signature_base64: Buffer.from("signature").toString("base64"),
    key_id_hint: "caller-key-1",
  });
  const handle = await signed.submit();
  assert.equal(handle.controlCapability._adapterHandleId(), 7);
  assert.equal(calls.find(([name]) => name === "submit")[1].signer_id, "caller-key-1");
  const awaited = await handle.awaitResult();
  assert.equal(awaited.terminalReceipt.receipt_ref, "opaque-receipt-ref");
  assert.equal(Object.hasOwn(awaited, "receipt"), false);
  assert.equal(Object.hasOwn(awaited, "terminal_receipt"), false);
  assert.equal((await handle.cancel("done")).terminal, true);
  assert.equal((await handle.refreshEvents()).events[0].kind, "completed");
  await handle.close();
  assert.deepEqual(calls.find(([name]) => name === "prepare")[2], { deadline_ms: 1000 });

  await assert.rejects(
    () => runtime.submitSigned(prepared),
    (error) => error instanceof sdk.SDKError && error.code === sdk.ErrorCode.INVALID_ARGUMENT,
  );
  await runtime.close();
});

test("invocation results expose terminalReceipt without legacy receipt fallback", async () => {
  const runtime = new sdk.RuntimeClient({
    invoke: () => JSON.stringify({
      ok: true,
      terminal_state: "Completed",
      terminal_receipt: { receipt_ref: "canonical-terminal" },
      receipt: { receipt_ref: "legacy-only" },
    }),
    prepare: (draftJSON) => preparedJSON(JSON.parse(Buffer.from(draftJSON).toString("utf8"))),
    submitSigned: () =>
      JSON.stringify({ handle_id: 7, state: "Running", terminal: false, events: [], result: null }),
    awaitHandle: () => JSON.stringify({
      ok: true,
      terminal_state: "Completed",
      receipt: { receipt_ref: "legacy-only" },
    }),
  });

  const invoked = await runtime.invoke(completeDraft());
  assert.equal(invoked.terminalReceipt.receipt_ref, "canonical-terminal");
  assert.equal(Object.hasOwn(invoked, "receipt"), false);
  assert.equal(Object.hasOwn(invoked, "terminal_receipt"), false);

  const prepared = await runtime.prepare(completeDraft());
  const handle = await prepared
    .signWithCallerSignature({
      algorithm: "ed25519",
      signature_base64: Buffer.from("signature").toString("base64"),
      key_id_hint: "caller-key-1",
    })
    .submit();
  const legacyOnly = await handle.awaitResult();
  assert.equal(Object.hasOwn(legacyOnly, "terminalReceipt"), false);
  assert.equal(Object.hasOwn(legacyOnly, "receipt"), false);

  await runtime.close();
});

test("public invocation handle JSON is observation-only", async () => {
  let reachedTransport = false;
  const runtime = new sdk.RuntimeClient({
    invoke: () => "{}",
    awaitHandle: () => {
      reachedTransport = true;
      return "{}";
    },
    cancelHandle: () => {
      reachedTransport = true;
      return "{}";
    },
    handleEvents: () => {
      reachedTransport = true;
      return "{}";
    },
    freeHandle: () => {
      reachedTransport = true;
    },
  });
  const handle = sdk.InvocationHandle.fromJSON(
    JSON.stringify({ handle_id: 7, state: "Submitted", terminal: false, events: [], result: null }),
  );
  assert.equal(handle.state, "Submitted");
  assert.equal(handle.toJSON().handle_id, 7);
  handle.bindRuntime(runtime);
  assert.throws(
    () => handle.controlCapability.constructor.fromHandleId(7)._adapterHandleId(),
    (error) => error instanceof sdk.SDKError && error.code === sdk.ErrorCode.INVALID_ARGUMENT,
  );
  assert.throws(
    () => new handle.controlCapability.constructor({ handle_id: 7, runtime_bound: true })._adapterHandleId(),
    (error) => error instanceof sdk.SDKError && error.code === sdk.ErrorCode.INVALID_ARGUMENT,
  );

  for (const action of [
    () => runtime.awaitResult(handle),
    () => runtime.cancel(handle, "done"),
    () => runtime.events(handle),
    () => runtime.closeHandle(handle),
    () => handle.awaitResult(),
  ]) {
    await assert.rejects(
      action,
      (error) => error instanceof sdk.SDKError && error.code === sdk.ErrorCode.INVALID_ARGUMENT,
    );
  }
  assert.equal(reachedTransport, false);
});

test("runtime events reject mismatched returned handle id", async () => {
  const runtime = new sdk.RuntimeClient({
    invoke: () => "{}",
    submitSigned: () =>
      JSON.stringify({ handle_id: 7, state: "Running", terminal: false, events: [], result: null }),
    handleEvents: () =>
      JSON.stringify({ handle_id: 8, state: "Running", terminal: false, events: [], result: null }),
  });
  const prepared = sdk.PreparedInvocation.fromJSON(preparedJSON(completeDraft().toJSON()));
  const signed = prepared.signWithCallerSignature({
    algorithm: "ed25519",
    signature_base64: Buffer.from("signature").toString("base64"),
    key_id_hint: "caller-key-1",
  }).bindRuntime(runtime);
  const handle = await signed.submit();

  await assert.rejects(
    () => runtime.events(handle),
    (error) => error instanceof sdk.SDKError && error.code === sdk.ErrorCode.INVALID_ARGUMENT,
  );
});

test("authority metadata is typed, delegated, and mutually exclusive", async () => {
  const delegation = delegationValue();
  const session = sessionValue();
  const authority = new sdk.AuthorityClient({
    mintDelegationProof: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      assert.equal(request.caller_ura, caller);
      return JSON.stringify({ metadata_value: delegation });
    },
    mintSessionAuthority: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      assert.equal(request.session_id, "session-1");
      return JSON.stringify({ metadata: { [sdk.SESSION_AUTHORITY_METADATA_KEY]: session } });
    },
  });

  const proof = await authority.mintDelegationProof({
    issuer_ura: "easynet:///r/example/user/alice",
    subject_ura: "easynet:///r/example/user/alice",
    caller_ura: caller,
    audience: callee,
    scopes: ["invoke"],
    issued_at_ms: 10,
    expires_at_ms: 20,
    metadata: { trace: "delegation" },
  });
  const sessionAuthority = await authority.mintSessionAuthority({
    issuer_ura: caller,
    session_id: "session-1",
    session_owner_user_id: "alice",
    creator_principal_id: caller,
    callee_ura: callee,
    subject_ura: "easynet:///r/example/user/alice",
    audience: callee,
    scopes: ["invoke"],
    allowed_actions: ["invoke"],
    allowed_followup_abilities: ["observe.health"],
    issued_at_ms: 10,
    expires_at_ms: 20,
    metadata: { trace: "session" },
  });
  assert.equal(proof.metadataValue, delegation);
  assert.equal(sessionAuthority.metadataValue, session);

  const authorized = new sdk.InvocationBuilder()
    .withCallerURA(caller)
    .withCalleeURA(callee)
    .withDescriptorRef(descriptor)
    .withSubjectURA(callee)
    .withNonceBase64(nonce)
    .withCausalContext({ form: "none" })
    .withJSONArgs({ probe: true })
    .withContentType("application/json")
    .withAuthorityMetadata(proof.metadata())
    .build();
  assert.equal(authorized.metadata[sdk.DELEGATION_METADATA_KEY], delegation);

  assert.throws(
    () =>
      new sdk.InvocationBuilder()
        .withCallerURA(caller)
        .withCalleeURA(callee)
        .withDescriptorRef(descriptor)
        .withSubjectURA(callee)
        .withNonceBase64(nonce)
        .withCausalContext({ form: "none" })
        .withJSONArgs({})
        .withContentType("application/json")
        .withMetadata({
          [sdk.DELEGATION_METADATA_KEY]: delegation,
          [sdk.SESSION_AUTHORITY_METADATA_KEY]: session,
        })
        .build(),
    (error) => error instanceof sdk.SDKError && error.code === sdk.ErrorCode.INVALID_ARGUMENT,
  );
  await authority.close();
});

test("stream and bidi state machines retain bounded history", async () => {
  let streamSequence = 0;
  const stream = new sdk.StreamHandle(
    {
      receive: () => JSON.stringify({ sequence: streamSequence++, kind: "data", terminal: false }),
    },
    { stream_id: "stream-1", max_buffered_events: 2 },
  );
  await stream.receive();
  await stream.receive();
  const overflow = await stream.receive();
  assert.equal(overflow.terminal, true);
  assert.equal(overflow.state, "Failed");
  assert.equal(overflow.error.details.reason, "callback_queue_overflow");
  assert.equal(stream.retainedEvents.length, 2);
  assert.equal(stream.terminalEvent().error.details.max_buffered_events, 2);

  const sent = [];
  const bidi = new sdk.BidiSession(
    {
      send: (frameJSON) => sent.push(JSON.parse(Buffer.from(frameJSON).toString("utf8"))),
      receive: () => JSON.stringify({ sequence: 1, kind: "data", terminal: false }),
    },
    { session_id: "bidi-1", max_buffered_frames: 1 },
  );
  await bidi.send({ sequence: 0, payload: { hello: true } });
  await assert.rejects(
    () => bidi.send({ sequence: 1, payload: {} }),
    (error) => error instanceof sdk.SDKError && error.code === sdk.ErrorCode.ADMISSION_DENIED,
  );
  assert.equal(sent.length, 1);
  assert.equal(bidi.terminalFrame().error.details.direction, "send");
});

test("typed errors preserve stable categories and source refs", () => {
  assert.equal(
    new sdk.SDKError({ code: sdk.ErrorCode.AUTHORITY_DENIED, stage: "admission", message: "denied" }).errorClass(),
    sdk.ErrorClass.ADMISSION,
  );
  assert.equal(
    new sdk.SDKError({ code: sdk.ErrorCode.ROUTE_UNAVAILABLE, stage: "routing", message: "missing" }).errorClass(),
    sdk.ErrorClass.AVAILABILITY,
  );
  assert.deepEqual(sdk.profileErrorDetails("health", { check: "runtime" }), {
    check: "runtime",
    profile: "health",
    source_ref: "node_sdk.profile.health",
  });
});
