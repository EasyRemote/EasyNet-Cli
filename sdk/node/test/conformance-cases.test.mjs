import assert from "node:assert/strict";
import test from "node:test";
import * as sdk from "../index.js";

const caller = "easynet:///r/example/agent/alice.sdk";
const callee = "easynet:///r/example/device/dev-a";
const descriptor = "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0";
const nonce = "AQIDBAUGBwgJCgsMDQ4PEA==";

function builder() {
  return new sdk.InvocationBuilder()
    .withCallerURA(caller)
    .withCalleeURA(callee)
    .withDescriptorRef(descriptor)
    .withSubjectURA(callee)
    .withNonceBase64(nonce)
    .withCausalContext({ form: "none" })
    .withJSONArgs({ probe: true })
    .withContentType("application/json");
}

function preparedJSON(draft) {
  return JSON.stringify({
    prepared_id: "prepared-1", request_id: "request-1", tuple: draft,
    signing_material: {
      algorithm: "ed25519", canonical_bytes_base64: Buffer.from("canonical").toString("base64"),
      args_digest_hex: "a".repeat(64), descriptor_ref: descriptor, nonce_base64: nonce,
      signed_fields: ["caller_ura", "callee_ura", "descriptor_ref", "subject_ura"],
      expires_at_unix_ms: 4102444800000,
    },
    descriptor_ref: descriptor, descriptor_hash_hex: "", schema_hash_hex: "",
    canonical_hash_hex: "", expires_at_unix_ms: 4102444800000, submit_ready: false,
  });
}

function runtime(overrides = {}) {
  return new sdk.RuntimeClient({
    invoke: () => JSON.stringify({ ok: true, terminal_state: "Completed" }),
    prepare: (raw) => preparedJSON(JSON.parse(Buffer.from(raw).toString("utf8"))),
    submitSigned: () => JSON.stringify({ handle_id: 7, state: "Running", terminal: false, events: [], result: null }),
    ...overrides,
  });
}

test("conformance version compatible accepts exact ABI", async () => {
  const client = new sdk.Client({ featureDiscovery: () => JSON.stringify({ abi_version: 5, sdk_version: "test" }) });
  assert.equal((await client.requireABI(5)).abiVersion, 5);
});

test("conformance version incompatible rejects mismatched ABI", async () => {
  const client = new sdk.Client({ featureDiscovery: () => JSON.stringify({ abi_version: 5, sdk_version: "test" }) });
  await assert.rejects(() => client.requireABI(4), (error) => error.code === sdk.ErrorCode.VERSION_MISMATCH);
});

test("conformance feature discovery projects runtime facts", async () => {
  const client = new sdk.Client({ featureDiscovery: () => JSON.stringify({ abi_version: 5, sdk_version: "test", symbols: { runtime_prepare: true } }) });
  assert.equal((await client.featureDiscovery()).symbols.runtime_prepare, true);
});

test("conformance health separates liveness and readiness", async () => {
  const health = new sdk.HealthClient({ runtimeHealth: () => JSON.stringify({ api_ready: true, invocation_ready: false, directory_ready: false, trust_ready: true, runtime_ready: false, abi_version: 5, version: "test", mismatch: null, diagnostics: [] }) });
  const state = await health.runtimeHealth();
  assert.equal(state.apiAlive(), true);
  assert.equal(state.ready(), false);
});

test("conformance typed error preserves stable JSON fields", () => {
  const error = new sdk.SDKError({ code: sdk.ErrorCode.TIMEOUT, stage: "execution", retry: sdk.RetryHint.SAFE, message: "timed out" });
  assert.equal(error.code, sdk.ErrorCode.TIMEOUT);
  assert.equal(error.stage, "execution");
  assert.equal(error.retry, sdk.RetryHint.SAFE);
});

test("conformance retry hint controls retryability", () => {
  assert.equal(new sdk.SDKError({ code: sdk.ErrorCode.TIMEOUT, stage: "execution", retry: sdk.RetryHint.SAFE, message: "timeout" }).retryable, true);
  assert.equal(new sdk.SDKError({ code: sdk.ErrorCode.INVALID_ARGUMENT, stage: "input", retry: sdk.RetryHint.NEVER, message: "bad" }).retryable, false);
  assert.throws(() => new sdk.SDKError({ code: sdk.ErrorCode.TIMEOUT, stage: "execution", retry: "later", message: "timeout" }));
});

test("conformance profile source reference is explicit", () => {
  assert.deepEqual(sdk.profileErrorDetails("health", { check: "runtime" }), { check: "runtime", profile: "health", source_ref: "node_sdk.profile.health" });
});

test("conformance authority metadata rejects ambiguity", () => {
  assert.throws(() => builder().withMetadata({ [sdk.DELEGATION_METADATA_KEY]: "a", [sdk.SESSION_AUTHORITY_METADATA_KEY]: "b" }).build(), (error) => error.code === sdk.ErrorCode.INVALID_ARGUMENT);
});

test("conformance builder handle is consumed by build", () => {
  const value = builder();
  value.inspect();
  value.build();
  assert.throws(() => value.inspect(), (error) => error.code === sdk.ErrorCode.INVALID_HANDLE);
});

test("conformance complete tuple rejects a missing field", () => {
  assert.throws(() => new sdk.InvocationBuilder().withCalleeURA(callee).withDescriptorRef(descriptor).withSubjectURA(callee).withNonceBase64(nonce).withCausalContext({ form: "none" }).withJSONArgs({}).withContentType("application/json").build(), (error) => error.code === sdk.ErrorCode.INVALID_ARGUMENT);
});

test("conformance prepare exposes delegated canonical material", async () => {
  const prepared = await runtime().prepare(builder().build());
  assert.equal(Buffer.from(prepared.signingMaterial.canonicalBytesBase64, "base64").toString(), "canonical");
  assert.equal(prepared.signingMaterial.descriptorRef, descriptor);
});

test("conformance prepared invocation cannot be submitted", async () => {
  const client = runtime();
  const prepared = await client.prepare(builder().build());
  await assert.rejects(() => client.submitSigned(prepared), (error) => error.code === sdk.ErrorCode.INVALID_ARGUMENT);
});

test("conformance presigned submit preserves caller signature", async () => {
  let submitted;
  const client = runtime({ submitSigned: (raw) => { submitted = JSON.parse(Buffer.from(raw).toString("utf8")); return JSON.stringify({ handle_id: 7, state: "Running", terminal: false, events: [], result: null }); } });
  const prepared = await client.prepare(builder().build());
  await client.submitSigned(prepared.signWithCallerSignature({ algorithm: "ed25519", signature_base64: Buffer.from("sig").toString("base64"), key_id_hint: "caller-key" }));
  assert.equal(submitted.signer_id, "caller-key");
});

test("conformance terminal handle state is monotonic", async () => {
  const handle = sdk.InvocationHandle.fromJSON(JSON.stringify({ handle_id: 7, state: "Completed", terminal: true, events: [{ sequence: 1, kind: "completed", state: "Completed", terminal: true }], result: { ok: true } }));
  assert.equal(handle.terminal, true);
  assert.equal(handle.events.filter((event) => event.terminal).length, 1);
});

test("conformance terminal receipt facts are explicit", async () => {
  const client = runtime({
    invoke: () => JSON.stringify({
      ok: true,
      terminal_state: "Completed",
      terminal_receipt: { receipt_ref: "canonical-terminal" },
      receipt: { receipt_ref: "legacy-only" },
    }),
  });
  const result = await client.invoke(builder().build());
  assert.equal(result.terminalReceipt.receipt_ref, "canonical-terminal");
  assert.equal(Object.hasOwn(result, "receipt"), false);
  assert.equal(Object.hasOwn(result, "terminal_receipt"), false);
});

test("conformance stream preserves order and terminal", async () => {
  const values = [JSON.stringify({ sequence: 0, kind: "data", terminal: false }), JSON.stringify({ sequence: 1, kind: "completed", state: "Completed", terminal: true })];
  const stream = new sdk.StreamHandle({ receive: () => values.shift() }, { stream_id: "s", max_buffered_events: 4 });
  assert.equal((await stream.receive()).sequence, 0);
  assert.equal((await stream.receive()).terminal, true);
});

test("conformance stream and bidi backpressure are bounded", async () => {
  let sequence = 0;
  const stream = new sdk.StreamHandle({ receive: () => JSON.stringify({ sequence: sequence++, kind: "data", terminal: false }) }, { stream_id: "s", max_buffered_events: 1 });
  await stream.receive();
  assert.equal((await stream.receive()).error.details.reason, "callback_queue_overflow");
  const bidi = new sdk.BidiSession({ send: () => {}, receive: () => JSON.stringify({ sequence: 1, kind: "data", terminal: false }) }, { session_id: "b", max_buffered_frames: 1 });
  await bidi.send({ sequence: 0 });
  await assert.rejects(() => bidi.send({ sequence: 1 }), (error) => error.code === sdk.ErrorCode.ADMISSION_DENIED);
});

test("conformance bidi close send does not cancel receive", async () => {
  let closed = false;
  const bidi = new sdk.BidiSession({ send: () => {}, closeSend: () => { closed = true; }, receive: () => JSON.stringify({ sequence: 1, kind: "data", terminal: false }) }, { session_id: "b", max_buffered_frames: 4 });
  await bidi.closeSend();
  assert.equal(closed, true);
  assert.equal((await bidi.receive()).terminal, false);
});

test("conformance bidi frame0 is required before runtime session entry", async () => {
  let opened = 0;
  const client = runtime({
    openBidi: () => {
      opened += 1;
      throw new Error("runtime session entry must not be reached");
    },
  });
  for (const streams of [undefined, []]) {
    await assert.rejects(
      () => client.openBidi(builder().build(), streams),
      (error) =>
        error instanceof sdk.SDKError &&
        error.code === sdk.ErrorCode.INVALID_ARGUMENT &&
        error.message.includes("bidi_streams must not be empty"),
    );
  }
  assert.equal(opened, 0);
});
