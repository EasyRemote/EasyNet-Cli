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

const productSymbols = [
  "AdminClient",
  "CompanionClient",
  "CompatibilityClient",
  "DirectoryClient",
  "MissionClient",
  "ReceiptClient",
  "SurfaceClient",
];

const declarations = await readFile(new URL("../index.d.ts", import.meta.url), "utf8");
for (const product of productSymbols) {
  assert.equal(Object.hasOwn(sdk, product), false, `${product} leaked through runtime exports`);
  assert.equal(declarations.includes(product), false, `${product} leaked through index.d.ts`);
}

const callerURA = "easynet:///r/example/agent/alice.sdk";
const calleeURA = "easynet:///r/example/device/dev-a";
const descriptorRef = "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0";
const agentBinding = (ura: string) => ({ ura, profile: "axon-strict-v2" });
const terminalReceipt = {
  receipt_ura: "easynet:///r/example/resource/runtime/invocation/inv-types/receipt/1",
  invocation_id: "inv-types",
  receipt_type: "completed",
  state: "Completed",
  index: 1,
  timestamp_unix_ms: 1_783_100_000_001,
  prev_receipt_hash_hex: "00".repeat(32),
  self_hash_hex: "01".repeat(32),
  cleanup_complete: true,
  caller_binding: agentBinding(callerURA),
  callee_binding: agentBinding(calleeURA),
  subject_binding: agentBinding(calleeURA),
  invocation_nonce_base64: "AQIDBAUGBwgJCgsMDQ4PEA==",
  causal_binding_kind: "none",
  causal_binding: { form: "none" },
  callee_signature: { algorithm: "ed25519", signature_base64: Buffer.alloc(64, 0x71).toString("base64") },
  signer_binding: agentBinding(calleeURA),
  authority_binding_kind: "self",
  authority_binding: { kind: "self", principal_ura: calleeURA },
  ability_binding: descriptorRef,
  subject_ref: { kind: 1, ura: calleeURA, profile: "axon-strict-v2" },
  descriptor_version: "1.0.0",
  schema_hash_hex: "11".repeat(32),
  impl_hash_hex: "22".repeat(32),
  runtime_env: "node-type-test",
  authority_proof: {
    proof_type: "self",
    binding_kind: "self",
    binding: { kind: "self", principal_ura: calleeURA },
    proof_payload_base64: "",
    proof_hash_hex: "55".repeat(32),
    issuer: agentBinding(calleeURA),
    signature: { algorithm: "ed25519", signature_base64: Buffer.alloc(64, 0x72).toString("base64") },
    admission_hook: "test.runtime.admission",
  },
  input_hash_hex: "33".repeat(32),
  output_hash_hex: "44".repeat(32),
  parent_receipts: [],
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
  .withNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==")
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
