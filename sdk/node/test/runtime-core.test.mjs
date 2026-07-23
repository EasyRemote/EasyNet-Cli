import assert from "node:assert/strict";
import { createHash } from "node:crypto";
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
  "ReceiptFilter",
  "ReceiptHistoryPage",
  "ReceiptListRequest",
  "RetryHint",
  "RuntimeCallContext",
  "RuntimeClient",
  "RuntimeHealth",
  "RuntimeReceipt",
  "SDKError",
  "SESSION_AUTHORITY_METADATA_KEY",
  "SessionAuthority",
  "SessionAuthorityRequest",
  "SessionHistoryOperations",
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
    subject_ura: callee,
    caller_ura: caller,
    audience: callee,
    scopes: ["observe.health"],
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

const sessionResourceValue = () =>
  authorityValue({
    issuer_ura: caller,
    session_id: "session-1",
    session_owner_user_id: "alice",
    creator_principal_id: caller,
    callee_ura: callee,
    subject_ura: "easynet:///r/example/resource/user.alice/session/session-1",
    audience: callee,
    scopes: ["observe.health"],
    allowed_actions: ["invoke"],
    allowed_followup_abilities: ["observe.health"],
    issued_at_ms: 10,
    expires_at_ms: 20,
  });

const historySessionValue = () =>
  authorityValue({
    issuer_ura: caller,
    session_id: "session-1",
    session_owner_user_id: "alice",
    creator_principal_id: caller,
    callee_ura: callee,
    subject_ura: "easynet:///r/example/resource/user.alice/session/session-1",
    audience: callee,
    scopes: ["invocation.history.list"],
    allowed_actions: ["invoke"],
    allowed_followup_abilities: ["invocation.history.list"],
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

const agentBinding = (ura) => ({ ura, profile: "axon-strict-v2" });

const canonicalRuntimeReceipt = (invocationId, receiptType, state, index) => {
  const proofPayload = Buffer.from("canonical-runtime-test-proof");
  return {
    receipt_ura: `easynet:///r/example/resource/runtime/invocation/${invocationId}/receipt/${index}`,
    invocation_id: invocationId,
    receipt_type: receiptType,
    state,
    index,
    timestamp_unix_ms: 1_783_100_000_000 + index,
    prev_receipt_hash_hex: "00".repeat(32),
    self_hash_hex: (index + 1).toString(16).padStart(64, "0"),
    cleanup_complete: !["admitted", "Admitted", "ADMITTED"].includes(state),
    caller_binding: agentBinding(caller),
    callee_binding: agentBinding(callee),
    subject_binding: agentBinding(callee),
    invocation_nonce_base64: nonce,
    causal_binding_kind: "none",
    causal_binding: { form: "none" },
    callee_signature: {
      algorithm: "ed25519",
      signature_base64: Buffer.alloc(64, 0x71).toString("base64"),
    },
    signer_binding: agentBinding(callee),
    authority_binding_kind: "self",
    authority_binding: { kind: "self", principal_ura: callee },
    ability_binding: descriptor,
    subject_ref: { kind: 1, ura: callee, profile: "axon-strict-v2" },
    descriptor_version: "1.0.0",
    schema_hash_hex: "11".repeat(32),
    impl_hash_hex: "22".repeat(32),
    runtime_env: "node-test",
    authority_proof: {
      proof_type: "self",
      binding_kind: "self",
      binding: { kind: "self", principal_ura: callee },
      proof_payload_base64: proofPayload.toString("base64"),
      proof_hash_hex: createHash("sha256").update(proofPayload).digest("hex"),
      issuer: agentBinding(callee),
      signature: {
        algorithm: "ed25519",
        signature_base64: Buffer.alloc(64, 0x72).toString("base64"),
      },
      admission_hook: "test.runtime.admission",
    },
    input_hash_hex: "33".repeat(32),
    output_hash_hex: "44".repeat(32),
    parent_receipts: [],
  };
};

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
        terminal_receipt: canonicalRuntimeReceipt("inv-direct", "completed", "Completed", 1),
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
        terminal_receipt: canonicalRuntimeReceipt("inv-await", "completed", "Completed", 1),
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
  assert.equal(invoked.terminalReceipt.invocation_id, "inv-direct");
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
  assert.equal(awaited.terminalReceipt.invocation_id, "inv-await");
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
  const terminal = canonicalRuntimeReceipt("inv-result", "completed", "Completed", 1);
  const runtime = new sdk.RuntimeClient({
    invoke: () => JSON.stringify({
      ok: true,
      terminal_state: "Completed",
      terminal_receipt: terminal,
    }),
    prepare: (draftJSON) => preparedJSON(JSON.parse(Buffer.from(draftJSON).toString("utf8"))),
    submitSigned: () =>
      JSON.stringify({ handle_id: 7, state: "Running", terminal: false, events: [], result: null }),
    awaitHandle: () => JSON.stringify({ ok: true, terminal_state: "Completed", receipt: terminal }),
  });

  const invoked = await runtime.invoke(completeDraft());
  assert.equal(invoked.terminalReceipt.invocation_id, "inv-result");
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
  await assert.rejects(
    () => handle.awaitResult(),
    (error) =>
      error instanceof sdk.SDKError
      && error.code === sdk.ErrorCode.INVALID_ARGUMENT
      && error.message.includes("retired receipt alias is not accepted"),
  );

  await runtime.close();
});

test("runtime receipt proof facts are mandatory", () => {
  const complete = canonicalRuntimeReceipt("inv-proof", "completed", "Completed", 1);
  const receipt = sdk.RuntimeReceipt.fromObject(complete);
  assert.equal(receipt.lifecycleState(), "COMPLETED");
  assert.equal(receipt.invocationId, "inv-proof");

  const missingProof = { ...complete };
  delete missingProof.authority_proof;
  assert.throws(
    () => sdk.RuntimeReceipt.fromObject(missingProof),
    (error) =>
      error instanceof sdk.SDKError
      && error.code === sdk.ErrorCode.INVALID_ARGUMENT
      && error.message.includes("authority_proof"),
  );
  assert.throws(
    () =>
      sdk.RuntimeReceipt.fromObject({
        ...complete,
        state: "Failed",
      }),
    (error) =>
      error instanceof sdk.SDKError
      && error.code === sdk.ErrorCode.INVALID_ARGUMENT
      && error.message.includes("receipt_type"),
  );
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

test("prepared invocation requires explicit top-level descriptor ref", () => {
  const value = JSON.parse(preparedJSON(completeDraft().toJSON()));
  delete value.descriptor_ref;

  assert.throws(
    () => sdk.PreparedInvocation.fromJSON(JSON.stringify(value)),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.INVALID_ARGUMENT &&
      error.message.includes("descriptor_ref"),
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
    subject_ura: callee,
    caller_ura: caller,
    audience: callee,
    scopes: ["observe.health"],
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
  assert.equal(sessionAuthority.sessionOwnerURA, "easynet:///r/example/user/alice");
  assert.equal(sessionAuthority.creatorPrincipalURA, caller);

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
        .withAuthorityMetadata(sessionAuthority.metadata())
        .build(),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.AUTHORITY_SUBJECT_MISMATCH &&
      error.stage === "authorize",
  );

  const userResourceSession = sdk.SessionAuthority.fromMetadata(sessionResourceValue());
  const resourceAuthorized = new sdk.InvocationBuilder()
    .withCallerURA(caller)
    .withCalleeURA(callee)
    .withDescriptorRef(descriptor)
    .withSubjectURA("easynet:///r/example/resource/user.alice/session/invocation_history")
    .withNonceBase64(nonce)
    .withCausalContext({ form: "none" })
    .withJSONArgs({})
    .withContentType("application/json")
    .withAuthorityMetadata(userResourceSession.metadata())
    .build();
  assert.equal(resourceAuthorized.metadata[sdk.SESSION_AUTHORITY_METADATA_KEY], userResourceSession.metadataValue);

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

test("authority client projects canonical principal URAs to current session wire", async () => {
  const payload = {
    issuer_ura: caller,
    session_id: "session-1",
    session_owner_user_id: "alice",
    creator_principal_id: "easynet:///r/example/authority",
    callee_ura: callee,
    subject_ura: "easynet:///r/example/resource/user.alice/session/session-1",
    audience: callee,
    scopes: ["device.observe.*"],
    allowed_actions: ["read"],
    allowed_followup_abilities: ["device.observe.health"],
    issued_at_ms: 1000,
    expires_at_ms: 2000,
  };
  let seenSession = null;
  const authority = new sdk.AuthorityClient({
    mintDelegationProof: () => JSON.stringify({ metadata_value: delegationValue() }),
    mintSessionAuthority: (requestJSON) => {
      seenSession = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      return JSON.stringify({
        metadata: {
          [sdk.SESSION_AUTHORITY_METADATA_KEY]: authorityValue(payload),
        },
      });
    },
  });

  const session = await authority.mintSessionAuthority({
    issuer_ura: caller,
    session_id: "session-1",
    session_owner_user_id: "",
    session_owner_ura: "easynet:///r/example/user/alice",
    creator_principal_id: "",
    creator_principal_ura: "easynet:///r/example/authority",
    callee_ura: callee,
    subject_ura: "easynet:///r/example/resource/user.alice/session/session-1",
    audience: callee,
    scopes: ["device.observe.*"],
    allowed_actions: ["read"],
    allowed_followup_abilities: ["device.observe.health"],
    issued_at_ms: 1000,
    expires_at_ms: 2000,
  });

  assert.equal(session.sessionOwnerURA, "easynet:///r/example/user/alice");
  assert.equal(session.creatorPrincipalURA, "easynet:///r/example/authority");
  assert.equal(seenSession.session_owner_user_id, "alice");
  assert.equal(seenSession.creator_principal_id, "easynet:///r/example/authority");
  assert.equal(Object.hasOwn(seenSession, "session_owner_ura"), false);
  assert.equal(Object.hasOwn(seenSession, "creator_principal_ura"), false);

  await authority.close();
});

test("authority client rejects conflicting canonical principal URAs", () => {
  const authority = new sdk.AuthorityClient({
    mintDelegationProof: () => JSON.stringify({ metadata_value: delegationValue() }),
    mintSessionAuthority: () => JSON.stringify({ metadata: {} }),
  });

  assert.throws(
    () =>
      new sdk.SessionAuthorityRequest({
        issuer_ura: caller,
        session_id: "session-1",
        session_owner_user_id: "bob",
        session_owner_ura: "easynet:///r/example/user/alice",
        creator_principal_id: caller,
        callee_ura: callee,
        subject_ura: "easynet:///r/example/resource/user.alice/session/session-1",
        audience: callee,
        scopes: ["device.observe.*"],
        allowed_actions: ["read"],
        allowed_followup_abilities: ["device.observe.health"],
        issued_at_ms: 1000,
        expires_at_ms: 2000,
      }),
    /session_owner_user_id must match session_owner_ura user id/,
  );

  assert.throws(
    () =>
      new sdk.SessionAuthorityRequest({
        issuer_ura: caller,
        session_id: "session-1",
        session_owner_user_id: "alice",
        creator_principal_id: caller,
        creator_principal_ura: "easynet:///r/example/authority",
        callee_ura: callee,
        subject_ura: "easynet:///r/example/resource/user.alice/session/session-1",
        audience: callee,
        scopes: ["device.observe.*"],
        allowed_actions: ["read"],
        allowed_followup_abilities: ["device.observe.health"],
        issued_at_ms: 1000,
        expires_at_ms: 2000,
      }),
    /creator_principal_id must match creator_principal_ura/,
  );

  authority.close();
});

test("authority metadata rejects all-zero session owners", () => {
  assert.throws(
    () => sdk.SessionAuthority.fromMetadata(
      authorityValue({
        issuer_ura: caller,
        session_id: "session-1",
        session_owner_user_id: "00000000-0000-0000-0000-000000000000",
        creator_principal_id: caller,
        callee_ura: callee,
        subject_ura: "easynet:///r/example/user/alice",
        audience: callee,
        scopes: ["invoke"],
        allowed_actions: ["invoke"],
        allowed_followup_abilities: ["observe.health"],
        issued_at_ms: 10,
        expires_at_ms: 20,
      }),
    ),
    /session_owner_user_id must not be all-zero/,
  );
});

test("authority metadata binds session subject to owner and session id", () => {
  assert.throws(
    () => sdk.SessionAuthority.fromMetadata(
      authorityValue({
        issuer_ura: caller,
        session_id: "session-1",
        session_owner_user_id: "alice",
        creator_principal_id: caller,
        callee_ura: callee,
        subject_ura: "easynet:///r/example/user/bob",
        audience: callee,
        scopes: ["invoke"],
        allowed_actions: ["invoke"],
        allowed_followup_abilities: ["observe.health"],
        issued_at_ms: 10,
        expires_at_ms: 20,
      }),
    ),
    /session authority user subject must match session_owner_user_id/,
  );

  assert.throws(
    () => sdk.SessionAuthority.fromMetadata(
      authorityValue({
        issuer_ura: caller,
        session_id: "session-1",
        session_owner_user_id: "alice",
        creator_principal_id: caller,
        callee_ura: callee,
        subject_ura: "easynet:///r/example/resource/user.alice/session/session-2",
        audience: callee,
        scopes: ["invoke"],
        allowed_actions: ["invoke"],
        allowed_followup_abilities: ["observe.health"],
        issued_at_ms: 10,
        expires_at_ms: 20,
      }),
    ),
    /session authority subject_ura owner\/session must match session_owner_user_id and session_id/,
  );

  assert.throws(
    () => sdk.SessionAuthority.fromMetadata(
      authorityValue({
        issuer_ura: caller,
        session_id: "session-1",
        session_owner_user_id: "teamalice",
        creator_principal_id: caller,
        callee_ura: callee,
        subject_ura: "easynet:///r/example/resource/user.team.alice/session/session-1",
        audience: callee,
        scopes: ["invoke"],
        allowed_actions: ["invoke"],
        allowed_followup_abilities: ["observe.health"],
        issued_at_ms: 10,
        expires_at_ms: 20,
      }),
    ),
    /session authority subject_ura must be a canonical user or session subject/,
  );

  assert.throws(
    () => new sdk.SessionAuthorityRequest({
      issuer_ura: caller,
      session_id: "session-1",
      session_owner_user_id: "alice",
      creator_principal_id: caller,
      callee_ura: callee,
      subject_ura: callee,
      audience: callee,
      scopes: ["invoke"],
      allowed_actions: ["invoke"],
      allowed_followup_abilities: ["observe.health"],
      issued_at_ms: 10,
      expires_at_ms: 20,
    }),
    /session authority subject_ura must be a canonical user or session subject/,
  );
});

test("session history preflight rejects authority subject mismatch before receipt provider", async () => {
  let providerCalls = 0;
  const history = new sdk.SessionHistoryOperations({
    list: () => {
      providerCalls += 1;
      return {
        records: [],
        next_cursor: "",
        limit: 50,
        source: "invocation.history.list",
      };
    },
  });

  const request = new sdk.ReceiptListRequest({
    call: {
      caller_ura: caller,
      callee_ura: callee,
      subject_ura: callee,
      metadata: {
        [sdk.SESSION_AUTHORITY_METADATA_KEY]: historySessionValue(),
      },
    },
    limit: 50,
  });

  await assert.rejects(
    () => history.list(request),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.AUTHORITY_SUBJECT_MISMATCH &&
      error.stage === "history" &&
      /session authority subject does not admit receipt query subject_ura/.test(error.message),
  );
  assert.equal(providerCalls, 0);
});

test("session history keeps subject filters as ledger predicates", async () => {
  let seenRequest = null;
  const history = new sdk.SessionHistoryOperations({
    list: (request) => {
      seenRequest = request;
      return JSON.stringify({
        records: [{ receipt_ura: "easynet:///r/example/resource/runtime/invocation/i1/receipt/1" }],
        next_cursor: "",
        limit: 25,
        source: "invocation.history.list",
      });
    },
  });

  const page = await history.list({
    call: {
      caller_ura: caller,
      callee_ura: callee,
      subject_ura: "easynet:///r/example/resource/user.alice/session/session-1",
      metadata: {
        [sdk.SESSION_AUTHORITY_METADATA_KEY]: historySessionValue(),
      },
    },
    filter: {
      caller_ura: caller,
      callee_ura: callee,
      subject_ura: callee,
    },
    limit: 25,
  });

  assert.equal(seenRequest.filter.subjectURA, callee);
  assert.equal(page.source, "invocation.history.list");
  assert.equal(page.records.length, 1);
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
    sdk.ErrorClass.ROUTING,
  );
  assert.equal(
    new sdk.SDKError({ code: sdk.ErrorCode.CALLER_IDENTITY_UNAVAILABLE, stage: "caller_identity", message: "missing identity" }).errorClass(),
    sdk.ErrorClass.PERMISSION,
  );
  assert.equal(
    new sdk.SDKError({ code: sdk.ErrorCode.CALLER_SIGNER_UNAVAILABLE, stage: "caller_identity", message: "missing signer" }).errorClass(),
    sdk.ErrorClass.ADMISSION,
  );
  assert.equal(
    new sdk.SDKError({ code: sdk.ErrorCode.DESCRIPTOR_NOT_FOUND, stage: "routing", message: "missing descriptor" }).errorClass(),
    sdk.ErrorClass.ROUTING,
  );
  assert.equal(
    new sdk.SDKError({ code: sdk.ErrorCode.RUNTIME_OFFLINE, stage: "transport", message: "offline" }).errorClass(),
    sdk.ErrorClass.AVAILABILITY,
  );
  assert.deepEqual(sdk.profileErrorDetails("health", { check: "runtime" }), {
    check: "runtime",
    profile: "health",
    source_ref: "node_sdk.profile.health",
  });
});
