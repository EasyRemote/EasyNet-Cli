import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import {
  DELEGATION_METADATA_KEY,
  AuthorityMetadata,
  InvocationBuilder,
  InvocationSignature,
  RuntimeClient,
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
  invoke: async () => JSON.stringify({ ok: true, terminal_receipt: { receipt_ref: "opaque" } }),
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
