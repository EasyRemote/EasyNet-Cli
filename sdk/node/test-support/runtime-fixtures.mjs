import { createHash } from "node:crypto";

export const TEST_CALLER = "easynet:///r/example/agent/alice.sdk";
export const TEST_DEVICE_SUBJECT = "easynet:///r/example/device/dev-a";
export const TEST_CALLEE = "easynet:///r/example/agent/device.dev-a.runtime-health";
export const TEST_DESCRIPTOR = "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0";
export const TEST_NONCE = "AQIDBAUGBwgJCgsMDQ4PEA==";

export const agentBinding = (ura) => ({ ura, profile: "axon-strict-v2" });

export const canonicalRuntimeReceipt = (invocationId, receiptType, state, index) => {
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
    payload_base64: "",
    payload_content_type: "application/json",
    cleanup_complete: !["admitted", "Admitted", "ADMITTED"].includes(state),
    caller_binding: agentBinding(TEST_CALLER),
    callee_binding: agentBinding(TEST_CALLEE),
    subject_binding: agentBinding(TEST_DEVICE_SUBJECT),
    invocation_nonce_base64: TEST_NONCE,
    causal_binding_kind: "none",
    causal_binding: { form: "none" },
    callee_signature: {
      algorithm: "ed25519",
      signature_base64: Buffer.alloc(64, 0x71).toString("base64"),
      key_id_hint: "callee-receipt-key",
    },
    signer_binding: agentBinding(TEST_CALLEE),
    authority_binding_kind: "self",
    authority_binding: { kind: "self", principal_ura: TEST_CALLEE },
    ability_binding: TEST_DESCRIPTOR,
    host_attestation_base64: "",
    usage: {
      tokens_in: 0,
      tokens_out: 0,
      duration_ms: 0,
      external_calls: 0,
    },
    subject_ref: { kind: 1, ura: TEST_DEVICE_SUBJECT, profile: "axon-strict-v2" },
    descriptor_version: "1.0.0",
    schema_hash_hex: "11".repeat(32),
    impl_hash_hex: "22".repeat(32),
    runtime_env: "node-test",
    authority_proof: {
      proof_type: "self",
      binding_kind: "self",
      binding: { kind: "self", principal_ura: TEST_CALLEE },
      proof_payload_base64: proofPayload.toString("base64"),
      proof_hash_hex: createHash("sha256").update(proofPayload).digest("hex"),
      issuer: agentBinding(TEST_CALLEE),
      signature: {
        algorithm: "ed25519",
        signature_base64: Buffer.alloc(64, 0x72).toString("base64"),
        key_id_hint: "authority-proof-key",
      },
      admission_hook: "test.runtime.admission",
    },
    input_hash_hex: "33".repeat(32),
    output_hash_hex: "44".repeat(32),
    parent_receipts: [],
  };
};
