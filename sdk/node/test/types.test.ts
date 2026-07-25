import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import {
  DELEGATION_METADATA_KEY,
  AuthorityMetadata,
  InvocationBuilder,
  InvocationSignature,
  RuntimeClient,
  RuntimeReceipt,
  type RuntimeTransport,
} from "../index.js";
import * as sdk from "../index.js";
import {
  TEST_CALLEE as calleeURA,
  TEST_CALLER as callerURA,
  TEST_DESCRIPTOR as descriptorRef,
  TEST_NONCE,
  canonicalRuntimeReceipt,
} from "../test-support/runtime-fixtures.mjs";

const downstreamProfileSymbols = [
  "WorkflowClient",
  "WorkflowTransport",
  "ApplicationLifecycleClient",
  "ApplicationDirectoryView",
  "ApplicationReceiptPage",
  "CompatibilityAdapter",
  "ConvenienceWrapperClient",
  "ProfileBundle",
];

const declarations = await readFile(new URL("../index.d.ts", import.meta.url), "utf8");
for (const symbol of downstreamProfileSymbols) {
  assert.equal(Object.hasOwn(sdk, symbol), false, `${symbol} leaked through runtime exports`);
  assert.equal(declarations.includes(symbol), false, `${symbol} leaked through index.d.ts`);
}

const terminalReceipt = {
  ...canonicalRuntimeReceipt("inv-types", "completed", "Completed", 1),
  runtime_env: "node-type-test",
};
const receipt = RuntimeReceipt.fromObject(terminalReceipt);
receipt.validateSummary();
const delegation = Buffer.from(
  JSON.stringify({
    payload: {
      issuer_ura: "easynet:///r/example/user/alice",
      subject_ura: calleeURA,
      caller_ura: callerURA,
      audience: calleeURA,
      scopes: ["observe.health"],
      issued_at_ms: 10,
      expires_at_ms: 20,
    },
    signature: Buffer.from("signature").toString("base64"),
  }),
).toString("base64");

const transport: RuntimeTransport = {
  invoke: async () => JSON.stringify({ ok: true, terminal_state: "Completed", terminal_receipt: terminalReceipt }),
  prepare: async (draftJSON) => JSON.stringify({
    prepared_id: "prepared-1",
    tuple: JSON.parse(new TextDecoder().decode(draftJSON)),
    signing_material: {
      canonical_bytes_base64: "Y2Fub25pY2Fs",
      args_digest_hex: "a".repeat(64),
      descriptor_ref: descriptorRef,
      expires_at_unix_ms: 4_102_444_800_000,
    },
  }),
  submitSigned: async () => JSON.stringify({ handle_id: 1, state: "Running", terminal: false }),
};

const draft = new InvocationBuilder()
  .withCallerURA(callerURA)
  .withCalleeURA(calleeURA)
  .withDescriptorRef(descriptorRef)
  .withSubjectURA(calleeURA)
  .withNonceBase64(TEST_NONCE)
  .withCausalContext({ form: "none" })
  .withJSONArgs({ probe: true })
  .withContentType("application/json")
  .withAuthorityMetadata(new AuthorityMetadata({
    kind: "delegation",
    key: DELEGATION_METADATA_KEY,
    value: delegation,
  }))
  .build();

const runtime = new RuntimeClient(transport);
const prepared = await runtime.prepare(draft);
const signed = prepared.signWithCallerSignature(new InvocationSignature({
  algorithm: "ed25519",
  signature_base64: "c2lnbmF0dXJl",
  key_id_hint: "caller-key-1",
}));
await runtime.submitSigned(signed);
