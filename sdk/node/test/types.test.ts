import {
  AuthorityMetadata,
  InvocationBuilder,
  InvocationSignature,
  RuntimeClient,
  type RuntimeTransport,
} from "../index.js";

// @ts-expect-error Product profiles are not part of the generic runtime SDK.
import { AdminClient } from "../index.js";

const transport: RuntimeTransport = {
  invoke: async () => JSON.stringify({ ok: true, receipt: { receipt_ref: "opaque" } }),
  prepare: async (draftJSON) => JSON.stringify({
    prepared_id: "prepared-1",
    tuple: JSON.parse(new TextDecoder().decode(draftJSON)),
    signing_material: {
      canonical_bytes_base64: "Y2Fub25pY2Fs",
      args_digest_hex: "a".repeat(64),
      descriptor_ref: "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
      expires_at_unix_ms: 4_102_444_800_000,
    },
  }),
  submitSigned: async () => JSON.stringify({ handle_id: 1, state: "Running", terminal: false }),
};

const draft = new InvocationBuilder()
  .withCallerURA("easynet:///r/example/agent/alice.sdk")
  .withCalleeURA("easynet:///r/example/device/dev-a")
  .withDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0")
  .withSubjectURA("easynet:///r/example/device/dev-a")
  .withNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==")
  .withCausalContext({ form: "none" })
  .withJSONArgs({ probe: true })
  .withContentType("application/json")
  .withAuthorityMetadata(new AuthorityMetadata({
    kind: "delegation",
    key: "x-easynet-delegation",
    value: "opaque-authority",
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

void AdminClient;
