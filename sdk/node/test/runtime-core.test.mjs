import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";
import * as sdk from "../index.js";
import {
  TEST_CALLEE as callee,
  TEST_CALLER as caller,
  TEST_DESCRIPTOR as descriptor,
  TEST_NONCE as nonce,
  canonicalRuntimeReceipt,
} from "../test-support/runtime-fixtures.mjs";

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
  "RuntimeAbilityClient",
  "RuntimeCallContext",
  "RuntimeClient",
  "RuntimeHealth",
  "RuntimeReceipt",
  "RuntimeReceiptProvider",
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
  "runtimeStateReadSubjectURA",
];

const downstreamProfileSymbols = [
  "WorkflowClient",
  "WorkflowTransport",
  "ApplicationLifecycleClient",
  "ApplicationDirectoryView",
  "ApplicationReceiptPage",
  "ApplicationEventClient",
  "HostIntegrationClient",
  "PublicationWorkflowClient",
  "TranslationLayer",
  "ConvenienceWrapperClient",
  "ProfileBundle",
  "ServiceLocator",
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

const delegationValue = (scopes = ["observe.health"]) =>
  authorityValue({
    issuer_ura: "easynet:///r/example/user/alice",
    subject_ura: callee,
    caller_ura: caller,
    audience: callee,
    scopes,
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

const historySessionValue = (override = {}) =>
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
    ...override,
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

const mutableAuthorityProof = (receipt) => {
  const proof = { ...receipt.authority_proof, binding: { ...receipt.authority_proof.binding } };
  receipt.authority_proof = proof;
  return proof;
};

const authorityBindingProofHashSelf = (principalURA) => {
  const principal = Buffer.from(principalURA, "utf8");
  const canonical = Buffer.concat([
    Buffer.from([0x01]),
    runtimeU32(principal.length),
    principal,
  ]);
  return createHash("sha256").update(canonical).digest("hex");
};

const runtimeLengthPrefixedText = (value) => {
  const encoded = Buffer.from(value, "utf8");
  return Buffer.concat([runtimeU32(encoded.length), encoded]);
};

const runtimeI64 = (value) => {
  const out = Buffer.alloc(8);
  out.writeBigInt64BE(BigInt(value));
  return out;
};

const authorityBindingProofHashSession = (binding) => {
  const signature = Buffer.from(binding.signature_base64, "base64");
  const canonical = Buffer.concat([
    Buffer.from([0x05]),
    runtimeLengthPrefixedText(binding.issuer_ura),
    runtimeLengthPrefixedText(binding.subject_ura),
    runtimeLengthPrefixedText(binding.session_id),
    runtimeU32(binding.scopes.length),
    ...binding.scopes.map(runtimeLengthPrefixedText),
    runtimeU32(binding.audiences.length),
    ...binding.audiences.map(runtimeLengthPrefixedText),
    runtimeI64(binding.issued_at_ms),
    runtimeI64(binding.expires_at_ms),
    runtimeU32(signature.length),
    signature,
  ]);
  return createHash("sha256").update(canonical).digest("hex");
};

const runtimeU32 = (value) => {
  const out = Buffer.alloc(4);
  out.writeUInt32BE(value);
  return out;
};

test("runtime package exports exactly the generic public surface", async () => {
  assert.deepEqual(Object.keys(sdk).sort(), expectedExports);

  const declarations = await readFile(new URL("../index.d.ts", import.meta.url), "utf8");
  for (const symbol of downstreamProfileSymbols) {
    assert.equal(declarations.includes(symbol), false, `${symbol} leaked through index.d.ts`);
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
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.VERSION_MISMATCH &&
      error.message.includes("runtime ABI version") &&
      !error.message.includes("daemon"),
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

  for (const retiredState of ["completed", "COMPLETED", "TIMED_OUT", " Completed "]) {
    assert.throws(
      () => sdk.RuntimeReceipt.fromObject({ ...complete, state: retiredState }),
      (error) =>
        error instanceof sdk.SDKError
        && error.code === sdk.ErrorCode.INVALID_ARGUMENT
        && error.message.includes("unknown receipt state"),
    );
  }
  assert.throws(
    () => sdk.RuntimeReceipt.fromObject({ ...complete, state: "Unspecified" }),
    (error) =>
      error instanceof sdk.SDKError
      && error.code === sdk.ErrorCode.INVALID_ARGUMENT
      && error.message.includes("runtime receipt lifecycle state must not be UNSPECIFIED"),
  );

  const missingProof = { ...complete };
  delete missingProof.authority_proof;
  assert.throws(
    () => sdk.RuntimeReceipt.fromObject(missingProof),
    (error) =>
      error instanceof sdk.SDKError
      && error.code === sdk.ErrorCode.INVALID_ARGUMENT
      && error.message.includes("authority_proof"),
  );
  for (const missingField of ["payload_base64", "payload_content_type", "host_attestation_base64", "usage"]) {
    const missingTopLevelFact = { ...complete };
    delete missingTopLevelFact[missingField];
    assert.throws(
      () => sdk.RuntimeReceipt.fromObject(missingTopLevelFact),
      (error) =>
        error instanceof sdk.SDKError
        && error.code === sdk.ErrorCode.INVALID_ARGUMENT
        && error.message.includes(`runtime receipt summary is missing runtime_receipt.${missingField}`),
    );
  }
  assert.throws(
    () => sdk.RuntimeReceipt.fromObject({
      ...complete,
      legacy_receipt_canonicalizer: "node-compatible-raw",
    }),
    (error) =>
      error instanceof sdk.SDKError
      && error.code === sdk.ErrorCode.INVALID_ARGUMENT
      && error.message.includes("runtime_receipt contains noncanonical field legacy_receipt_canonicalizer"),
  );
  const legacyAuthorityBinding = {
    ...complete,
    authority_binding: {
      ...complete.authority_binding,
      legacy_authority: "opaque",
    },
  };
  assert.throws(
    () => sdk.RuntimeReceipt.fromObject(legacyAuthorityBinding),
    (error) =>
      error instanceof sdk.SDKError
      && error.code === sdk.ErrorCode.INVALID_ARGUMENT
      && error.message.includes("authority_binding contains noncanonical field legacy_authority"),
  );
  const legacyAuthorityProof = { ...complete };
  mutableAuthorityProof(legacyAuthorityProof).legacy_proof_fact = "opaque";
  assert.throws(
    () => sdk.RuntimeReceipt.fromObject(legacyAuthorityProof),
    (error) =>
      error instanceof sdk.SDKError
      && error.code === sdk.ErrorCode.INVALID_ARGUMENT
      && error.message.includes("authority_proof contains noncanonical field legacy_proof_fact"),
  );
  const missingProofPayload = { ...complete };
  delete mutableAuthorityProof(missingProofPayload).proof_payload_base64;
  assert.throws(
    () => sdk.RuntimeReceipt.fromObject(missingProofPayload),
    (error) =>
      error instanceof sdk.SDKError
      && error.code === sdk.ErrorCode.INVALID_ARGUMENT
      && error.message.includes("runtime receipt summary is missing authority_proof.proof_payload_base64"),
  );
  const legacyProofIssuer = { ...complete };
  mutableAuthorityProof(legacyProofIssuer).issuer = {
    ...complete.callee_binding,
    legacy_profile: "opaque",
  };
  assert.throws(
    () => sdk.RuntimeReceipt.fromObject(legacyProofIssuer),
    (error) =>
      error instanceof sdk.SDKError
      && error.code === sdk.ErrorCode.INVALID_ARGUMENT
      && error.message.includes("authority_proof.issuer contains noncanonical field legacy_profile"),
  );
  assert.throws(
    () => sdk.RuntimeReceipt.fromObject({
      ...complete,
      causal_binding: { form: "none", legacy_parent: "opaque" },
    }),
    (error) =>
      error instanceof sdk.SDKError
      && error.code === sdk.ErrorCode.INVALID_ARGUMENT
      && error.message.includes("causal_binding contains noncanonical field legacy_parent"),
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

  const mismatchedProofHash = { ...complete };
  mutableAuthorityProof(mismatchedProofHash).proof_hash_hex = "ff".repeat(32);
  assert.throws(
    () => sdk.RuntimeReceipt.fromObject(mismatchedProofHash),
    (error) =>
      error instanceof sdk.SDKError
      && error.code === sdk.ErrorCode.INVALID_ARGUMENT
      && error.message.includes("authority_proof_hash_mismatch"),
  );

  const bindingHashProof = { ...complete };
  const proof = mutableAuthorityProof(bindingHashProof);
  proof.proof_payload_base64 = "";
  proof.proof_hash_hex = authorityBindingProofHashSelf(callee);
  assert.equal(sdk.RuntimeReceipt.fromObject(bindingHashProof).lifecycleState(), "COMPLETED");

  const missingProofSignature = { ...complete };
  delete mutableAuthorityProof(missingProofSignature).signature;
  assert.throws(
    () => sdk.RuntimeReceipt.fromObject(missingProofSignature),
    (error) =>
      error instanceof sdk.SDKError
      && error.code === sdk.ErrorCode.INVALID_ARGUMENT
      && error.message.includes("authority_proof.signature must be an object"),
  );

  const wrongIssuer = { ...complete };
  mutableAuthorityProof(wrongIssuer).issuer = {
    ura: "easynet:///r/example/device/other",
    profile: "axon-strict-v2",
  };
  assert.throws(
    () => sdk.RuntimeReceipt.fromObject(wrongIssuer),
    (error) =>
      error instanceof sdk.SDKError
      && error.code === sdk.ErrorCode.INVALID_ARGUMENT
      && error.message.includes("authority_proof issuer does not match callee_binding"),
  );

  for (const retiredProfile of ["axon-legacy-v1", "opaque"]) {
    const retiredCalleeProfile = {
      ...complete,
      callee_binding: {
        ...complete.callee_binding,
        profile: retiredProfile,
      },
    };
    assert.throws(
      () => sdk.RuntimeReceipt.fromObject(retiredCalleeProfile),
      (error) =>
        error instanceof sdk.SDKError
        && error.code === sdk.ErrorCode.INVALID_ARGUMENT
        && error.message.includes("callee_binding.profile is not canonical"),
    );
  }

  const hostedSignerWithoutAttestation = {
    ...complete,
    signer_binding: {
      ura: "easynet:///r/example/device/runtime-host",
      profile: "axon-strict-v2",
    },
  };
  assert.throws(
    () => sdk.RuntimeReceipt.fromObject(hostedSignerWithoutAttestation),
    (error) =>
      error instanceof sdk.SDKError
      && error.code === sdk.ErrorCode.INVALID_ARGUMENT
      && error.message.includes("hosted runtime receipt is missing host_attestation_base64"),
  );

  const selfSignerWithAttestation = {
    ...complete,
    host_attestation_base64: Buffer.alloc(64, 0x73).toString("base64"),
  };
  assert.throws(
    () => sdk.RuntimeReceipt.fromObject(selfSignerWithAttestation),
    (error) =>
      error instanceof sdk.SDKError
      && error.code === sdk.ErrorCode.INVALID_ARGUMENT
      && error.message.includes("self-signed runtime receipt must not carry host_attestation_base64"),
  );
});

test("runtime receipt projection is deep immutable", () => {
  const complete = canonicalRuntimeReceipt("inv-proof-immutable", "completed", "Completed", 1);
  const receipt = sdk.RuntimeReceipt.fromObject(complete);

  complete.authority_binding.legacy_authority = "post-validation-mutation";
  complete.authority_proof.binding.legacy_proof_fact = "post-validation-mutation";

  assert.equal(Object.isFrozen(receipt.raw), true);
  assert.equal(Object.isFrozen(receipt.raw.authority_binding), true);
  assert.equal(Object.isFrozen(receipt.raw.authority_proof), true);
  assert.equal(Object.isFrozen(receipt.raw.authority_proof.binding), true);

  const firstProjection = receipt.rawProjection();
  assert.equal(Object.hasOwn(firstProjection.authority_binding, "legacy_authority"), false);
  assert.equal(Object.hasOwn(firstProjection.authority_proof.binding, "legacy_proof_fact"), false);

  firstProjection.authority_binding.legacy_authority = "raw-projection-mutation";
  firstProjection.authority_proof.binding.legacy_proof_fact = "raw-projection-mutation";

  const secondProjection = receipt.rawProjection();
  assert.equal(Object.hasOwn(secondProjection.authority_binding, "legacy_authority"), false);
  assert.equal(Object.hasOwn(secondProjection.authority_proof.binding, "legacy_proof_fact"), false);
});

test("runtime receipt session authority facade uses generic fields", () => {
  const sessionBinding = {
    kind: "session",
    issuer_ura: "easynet:///r/example/agent/backend",
    subject_ura: "easynet:///r/example/agent/alice",
    session_id: "session-1",
    scopes: ["invoke"],
    audiences: [descriptor],
    issued_at_ms: 1,
    expires_at_ms: 2,
    signature_base64: Buffer.alloc(64, 0x73).toString("base64"),
  };
  const complete = canonicalRuntimeReceipt("inv-session-authority", "completed", "Completed", 1);
  complete.authority_binding_kind = "session";
  complete.authority_binding = sessionBinding;
  const proof = mutableAuthorityProof(complete);
  proof.proof_type = "session";
  proof.binding_kind = "session";
  proof.binding = { ...sessionBinding };
  proof.proof_payload_base64 = "";
  proof.proof_hash_hex = authorityBindingProofHashSession(sessionBinding);
  assert.equal(sdk.RuntimeReceipt.fromObject(complete).lifecycleState(), "COMPLETED");

  const retiredBinding = {
    kind: "session",
    backend_ura: "easynet:///r/example/agent/backend",
    user_ura: "easynet:///r/example/agent/alice",
    session_id: "session-1",
    scopes: ["invoke"],
    audiences: [descriptor],
    issued_at_ms: 1,
    expires_at_ms: 2,
    signature_base64: Buffer.alloc(64, 0x73).toString("base64"),
  };
  const retired = canonicalRuntimeReceipt("inv-retired-session-authority", "completed", "Completed", 1);
  retired.authority_binding_kind = "session";
  retired.authority_binding = retiredBinding;
  const retiredProof = mutableAuthorityProof(retired);
  retiredProof.proof_type = "session";
  retiredProof.binding_kind = "session";
  retiredProof.binding = { ...retiredBinding };
  retiredProof.proof_payload_base64 = "";
  assert.throws(
    () => sdk.RuntimeReceipt.fromObject(retired),
    (error) =>
      error instanceof sdk.SDKError
      && error.code === sdk.ErrorCode.INVALID_ARGUMENT
      && error.message.includes("authority_binding contains noncanonical field backend_ura"),
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

test("provider-managed signer policy requires custody facts", () => {
  for (const [label, policy, expected] of [
    [
      "missing signer_id",
      { mode: "provider_managed_signing", policy_ref: "policy/local" },
      "signer_id",
    ],
    [
      "blank signer_id",
      { mode: "provider_managed_signing", signer_id: " ", policy_ref: "policy/local" },
      "signer_id",
    ],
    [
      "missing policy_ref",
      { mode: "provider_managed_signing", signer_id: "signer-key-1" },
      "policy_ref",
    ],
    [
      "blank policy_ref",
      { mode: "provider_managed_signing", signer_id: "signer-key-1", policy_ref: " " },
      "policy_ref",
    ],
  ]) {
    const value = JSON.parse(preparedJSON(completeDraft().toJSON()));
    value.signing_material.signer_policy = policy;

    assert.throws(
      () => sdk.PreparedInvocation.fromJSON(JSON.stringify(value)),
      (error) =>
        error instanceof sdk.SDKError &&
        error.code === sdk.ErrorCode.INVALID_ARGUMENT &&
        error.message.includes("provider-managed signer_policy") &&
        error.message.includes(expected),
      label,
    );
  }
});

test("prepared invocation requires explicit expiry fact", () => {
  for (const [label, edit, expected] of [
    [
      "missing top-level expiry",
      (value) => {
        delete value.expires_at_unix_ms;
      },
      "expires_at_unix_ms",
    ],
    [
      "mismatched top-level expiry",
      (value) => {
        value.expires_at_unix_ms += 1;
      },
      "expires_at_unix_ms must match signing_material.expires_at_unix_ms",
    ],
  ]) {
    const value = JSON.parse(preparedJSON(completeDraft().toJSON()));
    edit(value);

    assert.throws(
      () => sdk.PreparedInvocation.fromJSON(JSON.stringify(value)),
      (error) =>
        error instanceof sdk.SDKError &&
        error.code === sdk.ErrorCode.INVALID_ARGUMENT &&
        error.message.includes(expected),
      label,
    );
  }
});

test("prepared invocation rejects request-id-only alias", () => {
  const value = JSON.parse(preparedJSON(completeDraft().toJSON()));
  delete value.prepared_id;

  assert.throws(
    () => sdk.PreparedInvocation.fromJSON(JSON.stringify(value)),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.INVALID_ARGUMENT &&
      error.message.includes("prepared_id is required"),
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
    .withSubjectURA("easynet:///r/example/resource/user.alice/session/session-1")
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
        .withSubjectURA("easynet:///r/example/resource/user.alice/session/invocation_history")
        .withNonceBase64(nonce)
        .withCausalContext({ form: "none" })
        .withJSONArgs({})
        .withContentType("application/json")
        .withAuthorityMetadata(userResourceSession.metadata())
        .build(),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.AUTHORITY_SUBJECT_MISMATCH &&
      error.stage === "authorize",
  );

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

test("runtime ability projection is canonical for authority scope admission", () => {
  for (const scope of [
    "observe.health",
    "easynet:///r/example/ability/device.dev-a.observe.health",
    "easynet:///r/example/ability/device.dev-a.*",
  ]) {
    const proof = sdk.DelegationProof.fromMetadata(delegationValue([scope]));
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
    assert.equal(authorized.metadata[sdk.DELEGATION_METADATA_KEY], proof.metadataValue);
  }

  const ownerQualifiedProof = sdk.DelegationProof.fromMetadata(
    delegationValue(["device.dev-a.observe.health"]),
  );
  assert.throws(
    () =>
      new sdk.InvocationBuilder()
        .withCallerURA(caller)
        .withCalleeURA(callee)
        .withDescriptorRef(descriptor)
        .withSubjectURA(callee)
        .withNonceBase64(nonce)
        .withCausalContext({ form: "none" })
        .withJSONArgs({ probe: true })
        .withContentType("application/json")
        .withAuthorityMetadata(ownerQualifiedProof.metadata())
        .build(),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.AUTHORITY_DENIED &&
      /delegation authority scopes do not admit invocation ability/.test(error.message),
  );

  const nestedDeviceCallee = "easynet:///r/example/resource/user.alice/archive/device/dev-a";
  const nestedDeviceProof = sdk.DelegationProof.fromMetadata(
    authorityValue({
      issuer_ura: "easynet:///r/example/user/alice",
      subject_ura: nestedDeviceCallee,
      caller_ura: caller,
      audience: nestedDeviceCallee,
      scopes: ["observe.health"],
      issued_at_ms: 10,
      expires_at_ms: 20,
    }),
  );
  assert.throws(
    () =>
      new sdk.InvocationBuilder()
        .withCallerURA(caller)
        .withCalleeURA(nestedDeviceCallee)
        .withDescriptorRef(descriptor)
        .withSubjectURA(nestedDeviceCallee)
        .withNonceBase64(nonce)
        .withCausalContext({ form: "none" })
        .withJSONArgs({ probe: true })
        .withContentType("application/json")
        .withAuthorityMetadata(nestedDeviceProof.metadata())
        .build(),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.AUTHORITY_DENIED &&
      /delegation authority scopes do not admit invocation ability/.test(error.message),
  );

  const proof = sdk.DelegationProof.fromMetadata(delegationValue(["observe.health"]));
  assert.throws(
    () =>
      new sdk.InvocationBuilder()
        .withCallerURA(caller)
        .withCalleeURA(callee)
        .withDescriptorRef("observe.health")
        .withSubjectURA(callee)
        .withNonceBase64(nonce)
        .withCausalContext({ form: "none" })
        .withJSONArgs({ probe: true })
        .withContentType("application/json")
        .withAuthorityMetadata(proof.metadata())
        .build(),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.INVALID_ARGUMENT &&
      error.message.includes("descriptor_ref must contain a canonical Ability URA"),
  );
});

test("runtime ability projection strips canonical authority owner prefix", () => {
  const subjectURA = "easynet:///r/example/resource/user.alice/invoke/namespace.resolve";
  const proof = sdk.DelegationProof.fromMetadata(
    authorityValue({
      issuer_ura: "easynet:///r/example/user/alice",
      subject_ura: subjectURA,
      caller_ura: caller,
      audience: "easynet:///r/example/authority",
      scopes: ["namespace.resolve"],
      issued_at_ms: 10,
      expires_at_ms: 20,
    }),
  );
  const authorized = new sdk.InvocationBuilder()
    .withCallerURA(caller)
    .withCalleeURA("easynet:///r/example/authority")
    .withDescriptorRef(
      "easynet:///r/example/ability/authority.namespace.resolve@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read",
    )
    .withSubjectURA(subjectURA)
    .withNonceBase64(nonce)
    .withCausalContext({ form: "none" })
    .withJSONArgs({})
    .withContentType("application/json")
    .withAuthorityMetadata(proof.metadata())
    .build();
  assert.equal(authorized.metadata[sdk.DELEGATION_METADATA_KEY], proof.metadataValue);
});

test("runtime ability projection rejects short scope for descriptor owner mismatch", () => {
  const proof = sdk.DelegationProof.fromMetadata(
    authorityValue({
      issuer_ura: "easynet:///r/example/user/alice",
      subject_ura: callee,
      caller_ura: caller,
      audience: callee,
      scopes: ["namespace.resolve"],
      issued_at_ms: 10,
      expires_at_ms: 20,
    }),
  );

  assert.throws(
    () =>
      new sdk.InvocationBuilder()
        .withCallerURA(caller)
        .withCalleeURA(callee)
        .withDescriptorRef(
          "easynet:///r/example/ability/authority.namespace.resolve@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read",
        )
        .withSubjectURA(callee)
        .withNonceBase64(nonce)
        .withCausalContext({ form: "none" })
        .withJSONArgs({})
        .withContentType("application/json")
        .withAuthorityMetadata(proof.metadata())
        .build(),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.AUTHORITY_DENIED &&
      error.message.includes("delegation authority scopes do not admit invocation ability"),
  );
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
    () =>
      sdk.SessionAuthority.fromMetadata(
        authorityValue({
          issuer_ura: caller,
          session_id: "invocation_history",
          session_owner_user_id: "alice",
          creator_principal_id: caller,
          callee_ura: callee,
          subject_ura: "easynet:///r/example/resource/user.alice/session/invocation_history",
          audience: callee,
          scopes: ["invoke"],
          allowed_actions: ["invoke"],
          allowed_followup_abilities: ["observe.health"],
          issued_at_ms: 10,
          expires_at_ms: 20,
        }),
      ),
    /session authority session_id is not canonical/,
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

test("runtime authority rejects path-substring owner subject before dispatch", () => {
  assert.throws(
    () =>
      new sdk.InvocationBuilder()
        .withCallerURA(caller)
        .withCalleeURA(callee)
        .withDescriptorRef(descriptor)
        .withSubjectURA("easynet:///r/example/device/dev-a/resource/user.alice/runtime-state/read")
        .withNonceBase64(nonce)
        .withCausalContext({ form: "none" })
        .withJSONArgs({ probe: true })
        .withContentType("application/json")
        .withMetadata({
          [sdk.SESSION_AUTHORITY_METADATA_KEY]: sessionValue(),
        })
        .build(),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.AUTHORITY_SUBJECT_MISMATCH &&
      /session authority subject does not admit invocation subject_ura/.test(error.message),
  );
});

test("public invocation builder rejects receipt history descriptor before dispatch", () => {
  const historyDescriptor =
    "easynet:///r/example/ability/device.dev-a.invocation.history.list@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read";

  assert.throws(
    () =>
      new sdk.InvocationBuilder()
        .withCallerURA(caller)
        .withCalleeURA(callee)
        .withDescriptorRef(historyDescriptor)
        .withSubjectURA(callee)
        .withNonceBase64(nonce)
        .withCausalContext({ form: "none" })
        .withJSONArgs({ probe: true })
        .withContentType("application/json")
        .build(),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.INVALID_ARGUMENT &&
      /runtime governance read ability `invocation\.history\.list`/.test(error.message) &&
      /SessionHistoryOperations/.test(error.message),
  );
});

test("public invocation builder rejects runtime catalogue descriptor before dispatch", () => {
  const catalogueDescriptor =
    "easynet:///r/example/ability/authority.meta.list_abilities@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read";

  assert.throws(
    () =>
      new sdk.InvocationBuilder()
        .withCallerURA(caller)
        .withCalleeURA("easynet:///r/example/authority")
        .withDescriptorRef(catalogueDescriptor)
        .withSubjectURA("easynet:///r/example/authority")
        .withNonceBase64(nonce)
        .withCausalContext({ form: "none" })
        .withJSONArgs({ scope: "realm" })
        .withContentType("application/json")
        .build(),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.INVALID_ARGUMENT &&
      /runtime governance read ability `meta\.list_abilities`/.test(error.message) &&
      /catalogue provider/.test(error.message),
  );
});

test("runtime ability public path rejects receipt history before descriptor resolution", async () => {
  let resolverCalls = 0;
  const runtime = new sdk.RuntimeClient({
    invoke: () => {
      throw new Error("invoke must not run");
    },
    resolveDescriptorRef: () => {
      resolverCalls += 1;
      return descriptor;
    },
  });
  const ability = new sdk.RuntimeAbilityClient(runtime);

  await assert.rejects(
    () =>
      ability.build(
        {
          caller_ura: caller,
          callee_ura: callee,
          subject_ura: callee,
          nonce_base64: nonce,
          causal_context: { form: "none" },
        },
        "invocation.history.list",
        { limit: 5 },
      ),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.INVALID_ARGUMENT &&
      /RuntimeReceiptProvider/.test(error.message),
  );
  assert.equal(resolverCalls, 0);
});

test("runtime receipt provider uses governance descriptor provider and complete tuple", async () => {
  const calls = [];
  const historyDescriptor =
    "easynet:///r/example/ability/device.dev-a.invocation.history.list@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read";
  const runtime = new sdk.RuntimeClient({
    resolveDescriptorRef: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      calls.push(["resolve", request]);
      return JSON.stringify({ descriptor_ref: historyDescriptor });
    },
    invoke: (draftJSON) => {
      const draft = JSON.parse(Buffer.from(draftJSON).toString("utf8"));
      calls.push(["invoke", draft]);
      return JSON.stringify({
        ok: true,
        terminal_state: "Completed",
        output: {
          records: [{ request_id: "req-1" }],
          next_cursor: "",
          ledger_ura: "easynet:///r/example/resource/device.dev-a/billing/invocations",
        },
        terminal_receipt: canonicalRuntimeReceipt("inv-history", "completed", "Completed", 1),
      });
    },
  });
  const history = new sdk.SessionHistoryOperations(
    new sdk.RuntimeReceiptProvider(new sdk.RuntimeAbilityClient(runtime)),
  );

  const page = await history.list({
    call: {
      caller_ura: caller,
      callee_ura: callee,
      subject_ura: callee,
      nonce_base64: nonce,
      causal_context: { form: "none" },
      metadata: {
        [sdk.DELEGATION_METADATA_KEY]: delegationValue(["invocation.history.*"]),
      },
    },
    limit: 5,
  });

  assert.equal(page.records.length, 1);
  assert.equal(page.source, "easynet:///r/example/resource/device.dev-a/billing/invocations");
  assert.deepEqual(calls[0], [
    "resolve",
    {
      callee_ura: callee,
      ability: "invocation.history.list",
      call_mode: "rpc",
      caller_ura: caller,
      subject_ura: callee,
      provider: "receipt_history",
    },
  ]);
  assert.equal(calls[1][1].descriptor_ref, historyDescriptor);
  assert.equal(calls[1][1].caller_ura, caller);
  assert.equal(calls[1][1].callee_ura, callee);
  assert.equal(calls[1][1].subject_ura, callee);
  assert.equal(calls[1][1].nonce_base64, nonce);
  assert.deepEqual(calls[1][1].causal_context, { form: "none" });
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
      subject_ura: sdk.runtimeStateReadSubjectURA("example", "alice"),
      nonce_base64: nonce,
      causal_context: { form: "none" },
      metadata: {
        [sdk.SESSION_AUTHORITY_METADATA_KEY]: historySessionValue({
          session_owner_user_id: "bob",
          subject_ura: "easynet:///r/example/resource/user.bob/session/session-1",
        }),
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

test("session history preflight requires complete call context before authority checks", async () => {
  for (const omitted of ["nonce_base64", "causal_context"]) {
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
    const call = {
      caller_ura: caller,
      callee_ura: callee,
      subject_ura: sdk.runtimeStateReadSubjectURA("example", "alice"),
      nonce_base64: nonce,
      causal_context: { form: "none" },
      metadata: {
        [sdk.SESSION_AUTHORITY_METADATA_KEY]: historySessionValue(),
      },
    };
    delete call[omitted];

    const request = new sdk.ReceiptListRequest({ call, limit: 50 });

    await assert.rejects(
      () => history.list(request),
      (error) =>
        error instanceof sdk.SDKError &&
        error.code === sdk.ErrorCode.INVALID_INVOCATION &&
        error.stage === "history" &&
        error.message === `${omitted} is required`,
    );
    assert.equal(providerCalls, 0);
  }
});

test("session history preflight rejects path-substring owner subject before receipt provider", async () => {
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
      subject_ura: "easynet:///r/example/device/dev-a/resource/user.alice/runtime-state/read",
      nonce_base64: nonce,
      causal_context: { form: "none" },
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
      error.code === sdk.ErrorCode.INVALID_INVOCATION &&
      error.stage === "history" &&
      /runtime-state read subject/.test(error.message),
  );
  assert.equal(providerCalls, 0);
});

test("session history preflight rejects retired session subject before receipt provider", async () => {
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
      subject_ura: "easynet:///r/example/resource/user.alice/session/invocation_history",
      nonce_base64: nonce,
      causal_context: { form: "none" },
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
      error.code === sdk.ErrorCode.INVALID_INVOCATION &&
      error.stage === "history" &&
      /runtime-state read subject/.test(error.message),
  );
  assert.equal(providerCalls, 0);
});

test("session history preflight accepts exact runtime-owner subject with delegation authority", async () => {
  let seenRequest = null;
  const history = new sdk.SessionHistoryOperations({
    list: (request) => {
      seenRequest = request;
      return {
        records: [],
        next_cursor: "",
        limit: 50,
        source: "invocation.history.list",
      };
    },
  });
  const deviceSubject = callee;

  const page = await history.list({
    call: {
      caller_ura: caller,
      callee_ura: callee,
      subject_ura: deviceSubject,
      nonce_base64: nonce,
      causal_context: { form: "none" },
      metadata: {
        [sdk.DELEGATION_METADATA_KEY]: delegationValue(["invocation.history.*"]),
      },
    },
    limit: 50,
  });

  assert.equal(seenRequest.call.subjectURA, deviceSubject);
  assert.equal(page.source, "invocation.history.list");
});

test("session history preflight rejects non-callee runtime-owner subject before provider", async () => {
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

  await assert.rejects(
    () =>
      history.list({
        call: {
          caller_ura: caller,
          callee_ura: callee,
          subject_ura: "easynet:///r/example/device/dev-b",
          nonce_base64: nonce,
          causal_context: { form: "none" },
          metadata: {
            [sdk.DELEGATION_METADATA_KEY]: delegationValue(["invocation.history.*"]),
          },
        },
        limit: 50,
      }),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.INVALID_INVOCATION &&
      error.stage === "history" &&
      /callee runtime-owner subject/.test(error.message),
  );
  assert.equal(providerCalls, 0);
});

test("session history preflight rejects runtime-owner subject with session authority before provider", async () => {
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

  await assert.rejects(
    () =>
      history.list({
        call: {
          caller_ura: caller,
          callee_ura: callee,
          subject_ura: callee,
          nonce_base64: nonce,
          causal_context: { form: "none" },
          metadata: {
            [sdk.SESSION_AUTHORITY_METADATA_KEY]: historySessionValue(),
          },
        },
        limit: 50,
      }),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.AUTHORITY_DENIED &&
      error.stage === "history" &&
      /session authority cannot authorize a runtime-owner receipt history subject/.test(error.message),
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
      subject_ura: sdk.runtimeStateReadSubjectURA("example", "alice"),
      nonce_base64: nonce,
      causal_context: { form: "none" },
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

test("receipt filter rejects retired agent_ura alias", () => {
  assert.throws(
    () => new sdk.ReceiptFilter({ agent_ura: callee }),
    /agent_ura is not a runtime field/,
  );
});

test("runtime-state read subject helper builds user-owned resource subject", () => {
  assert.equal(
    sdk.runtimeStateReadSubjectURA("example", "alice"),
    "easynet:///r/example/resource/user.alice/runtime-state/read",
  );
});

test("runtime-state read subject helper rejects all-zero user before device fallback", () => {
  assert.throws(
    () => sdk.runtimeStateReadSubjectURA("example", "00000000-0000-0000-0000-000000000000"),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.INVALID_ARGUMENT &&
      error.stage === "build" &&
      /user_id must not be all-zero/.test(error.message),
  );
});

test("runtime-state read subject predicate rejects all-zero owner before history preflight", () => {
  assert.throws(
    () =>
      new sdk.ReceiptListRequest({
        call: {
          caller_ura: caller,
          callee_ura: callee,
          subject_ura:
            "easynet:///r/example/resource/user.00000000-0000-0000-0000-000000000000/runtime-state/read",
          metadata: {
            [sdk.SESSION_AUTHORITY_METADATA_KEY]: historySessionValue(),
          },
        },
        limit: 50,
      }),
    /subject_ura must not be all-zero/,
  );
});

test("runtime-state read subject helper rejects non-canonical realm and user segments", () => {
  assert.throws(
    () => sdk.runtimeStateReadSubjectURA("example/tenant", "alice"),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.INVALID_ARGUMENT &&
      error.stage === "build" &&
      /realm is not canonical/.test(error.message),
  );
  assert.throws(
    () => sdk.runtimeStateReadSubjectURA("example", "alice/sdk"),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.INVALID_ARGUMENT &&
      error.stage === "build" &&
      /user_id is not canonical/.test(error.message),
  );
  assert.throws(
    () => sdk.runtimeStateReadSubjectURA("example?tenant", "alice"),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.INVALID_ARGUMENT &&
      error.stage === "build" &&
      /realm is not canonical/.test(error.message),
  );
  assert.throws(
    () => sdk.runtimeStateReadSubjectURA("example", "alice#sdk"),
    (error) =>
      error instanceof sdk.SDKError &&
      error.code === sdk.ErrorCode.INVALID_ARGUMENT &&
      error.stage === "build" &&
      /user_id is not canonical/.test(error.message),
  );
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

test("stream and bidi terminality ignore retired event-kind aliases", async () => {
  const streamFrames = [
    { sequence: 1, frame_type: "terminal", state: "Running", terminal: false },
    { sequence: 2, kind: "terminal", state: "Completed", terminal: false },
  ];
  const stream = new sdk.StreamHandle(
    {
      receive: () => JSON.stringify(streamFrames.shift()),
    },
    { stream_id: "stream-alias-cutover", max_buffered_events: 4 },
  );

  const aliasStreamEvent = await stream.receive();
  assert.equal(aliasStreamEvent.frame_type, "terminal");
  assert.equal(stream.terminal, false);
  assert.equal(stream.terminalEvent(), null);

  await stream.receive();
  assert.equal(stream.terminal, true);

  const bidiFrames = [
    { sequence: 1, event_type: "terminal", state: "Running", terminal: false },
    { sequence: 2, kind: "completed", state: "Completed", terminal: false },
  ];
  const bidi = new sdk.BidiSession(
    {
      send: () => {},
      receive: () => JSON.stringify(bidiFrames.shift()),
    },
    { session_id: "bidi-alias-cutover", max_buffered_frames: 4 },
  );

  const aliasBidiFrame = await bidi.receive();
  assert.equal(aliasBidiFrame.event_type, "terminal");
  assert.equal(bidi.terminal, false);
  assert.equal(bidi.terminalFrame(), null);

  await bidi.receive();
  assert.equal(bidi.terminal, true);
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
