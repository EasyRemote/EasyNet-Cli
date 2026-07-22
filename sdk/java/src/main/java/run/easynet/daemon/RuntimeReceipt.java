package run.easynet.daemon;

import java.util.Base64;
import java.util.HexFormat;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public final class RuntimeReceipt {
  private static final HexFormat HEX = HexFormat.of();

  private final Map<String, Object> raw;
  private final String invocationId;
  private final String receiptType;
  private final String state;

  private RuntimeReceipt(Map<String, Object> raw) {
    this.raw = Map.copyOf(requireObject(raw, "runtime receipt"));
    this.invocationId = requiredString(this.raw, "invocation_id");
    this.receiptType = requiredString(this.raw, "receipt_type");
    this.state = requiredString(this.raw, "state");
    validateSummary();
  }

  public static RuntimeReceipt fromMap(Map<String, Object> raw) {
    return new RuntimeReceipt(raw);
  }

  public Map<String, Object> raw() {
    return raw;
  }

  public String invocationId() {
    return invocationId;
  }

  public String receiptType() {
    return receiptType;
  }

  public String state() {
    return state;
  }

  public String lifecycleState() {
    return canonicalLifecycleState(state);
  }

  public Map<String, Object> rawProjection() {
    return raw;
  }

  private void validateSummary() {
    if (invocationId.isBlank()) {
      throw SDKError.validation(
          "runtime_receipt", "runtime receipt summary is missing invocation_id");
    }
    if (receiptType.isBlank()) {
      throw SDKError.validation(
          "runtime_receipt", "runtime receipt summary is missing receipt_type");
    }
    String lifecycleState = canonicalLifecycleState(state);
    if (lifecycleState.equals("UNSPECIFIED")) {
      throw SDKError.validation(
          "runtime_receipt", "runtime receipt lifecycle state must not be UNSPECIFIED");
    }
    if (!receiptType.equals(canonicalReceiptType(lifecycleState))) {
      throw SDKError.validation(
          "runtime_receipt", "runtime receipt receipt_type does not match its lifecycle state");
    }
    receiptHash(raw, "prev_receipt_hash_hex", true);
    receiptHash(raw, "self_hash_hex", false);
    validateProofFacts(raw);
  }

  private static void validateProofFacts(Map<String, Object> raw) {
    requireAgentBinding(raw.get("caller_binding"), "caller_binding");
    requireAgentBinding(raw.get("callee_binding"), "callee_binding");
    requireSubjectBinding(raw.get("subject_binding"), "subject_binding");
    base64Bytes(
        requiredString(raw, "invocation_nonce_base64"),
        "invocation_nonce_base64",
        16,
        false);

    String causalKind = requiredString(raw, "causal_binding_kind");
    Map<String, Object> causalBinding =
        requireObject(raw.get("causal_binding"), "causal_binding");
    validateCausalBinding(causalKind, causalBinding);

    requireSignature(raw.get("callee_signature"), "callee_signature");
    requireAgentBinding(raw.get("signer_binding"), "signer_binding");

    String authorityKind = requiredString(raw, "authority_binding_kind");
    Map<String, Object> authorityBinding =
        requireAuthorityBinding(raw.get("authority_binding"), "authority_binding");
    if (!authorityKind.equals(requiredString(authorityBinding, "kind"))) {
      throw SDKError.validation(
          "runtime_receipt",
          "runtime receipt authority_binding kind does not match authority_binding_kind");
    }

    requiredString(raw, "ability_binding");
    requireEntityRef(raw.get("subject_ref"), "subject_ref");
    requiredString(raw, "descriptor_version");
    receiptHash(raw, "schema_hash_hex", false);
    receiptHash(raw, "impl_hash_hex", false);
    requiredString(raw, "runtime_env");

    Map<String, Object> authorityProof =
        requireObject(raw.get("authority_proof"), "authority_proof");
    requiredString(authorityProof, "proof_type");
    String proofBindingKind = requiredString(authorityProof, "binding_kind");
    if (!proofBindingKind.equals(authorityKind)) {
      throw SDKError.validation(
          "runtime_receipt",
          "runtime receipt authority_proof binding_kind does not match authority_binding_kind");
    }
    Map<String, Object> proofBinding =
        requireAuthorityBinding(authorityProof.get("binding"), "authority_proof.binding");
    if (!proofBinding.equals(authorityBinding)) {
      throw SDKError.validation(
          "runtime_receipt",
          "runtime receipt authority_proof binding does not match authority_binding");
    }
    base64Bytes(
        optionalString(authorityProof.get("proof_payload_base64")),
        "authority_proof.proof_payload_base64",
        0,
        true);
    receiptHash(authorityProof, "proof_hash_hex", false);
    requireAgentBinding(authorityProof.get("issuer"), "authority_proof.issuer");
    requireSignature(authorityProof.get("signature"), "authority_proof.signature");

    receiptHash(raw, "input_hash_hex", false);
    receiptHash(raw, "output_hash_hex", false);
    requireParentReceipts(raw.get("parent_receipts"));
  }

  private static String canonicalLifecycleState(String value) {
    return switch (value.trim()) {
      case "accepted", "Accepted", "ACCEPTED" -> "ACCEPTED";
      case "admitted", "Admitted", "ADMITTED" -> "ADMITTED";
      case "dispatched", "Dispatched", "DISPATCHED" -> "DISPATCHED";
      case "running", "Running", "RUNNING" -> "RUNNING";
      case "completed", "Completed", "COMPLETED" -> "COMPLETED";
      case "failed", "Failed", "FAILED" -> "FAILED";
      case "timed_out", "TimedOut", "TIMED_OUT" -> "TIMED_OUT";
      case "cancelled", "Cancelled", "CANCELLED" -> "CANCELLED";
      case "unspecified", "Unspecified", "UNSPECIFIED" -> "UNSPECIFIED";
      default -> throw SDKError.validation("runtime_receipt", "unknown receipt state " + value);
    };
  }

  private static String canonicalReceiptType(String lifecycleState) {
    return switch (lifecycleState) {
      case "ACCEPTED" -> "accepted";
      case "ADMITTED" -> "admitted";
      case "DISPATCHED" -> "dispatched";
      case "RUNNING" -> "running";
      case "COMPLETED" -> "completed";
      case "FAILED" -> "failed";
      case "TIMED_OUT" -> "timed_out";
      case "CANCELLED" -> "cancelled";
      default -> "";
    };
  }

  private static void validateCausalBinding(String kind, Map<String, Object> binding) {
    String form = requiredString(binding, "form");
    if (!form.equals(kind)) {
      throw SDKError.validation(
          "runtime_receipt",
          "runtime receipt causal_binding form does not match causal_binding_kind");
    }
    switch (form) {
      case "none" -> {}
      case "scalar" -> requireReceiptRef(binding.get("receipt"), "causal_binding.receipt");
      case "list" -> {
        List<Object> prior = requireList(binding.get("prior"), "causal_binding.prior");
        if (prior.isEmpty()) {
          throw SDKError.validation(
              "runtime_receipt", "causal_binding.prior must be a non-empty array");
        }
        for (int i = 0; i < prior.size(); i++) {
          requireReceiptRef(prior.get(i), "causal_binding.prior[" + i + "]");
        }
      }
      case "merkle" -> {
        receiptHash(binding, "root_hex", false);
        requiredString(binding, "proof_ura");
      }
      default ->
          throw SDKError.validation("runtime_receipt", "unsupported causal_binding form " + form);
    }
  }

  private static void requireReceiptRef(Object value, String field) {
    Map<String, Object> ref = requireObject(value, field);
    receiptHash(ref, "receipt_hash_hex", false);
    requiredString(ref, "receipt_ura");
  }

  private static void requireParentReceipts(Object value) {
    List<Object> parents = requireList(value, "parent_receipts");
    for (int i = 0; i < parents.size(); i++) {
      requireReceiptRef(parents.get(i), "parent_receipts[" + i + "]");
    }
  }

  private static void requireAgentBinding(Object value, String field) {
    Map<String, Object> binding = requireObject(value, field);
    requiredString(binding, "ura");
    requiredString(binding, "profile");
  }

  private static void requireSubjectBinding(Object value, String field) {
    requireAgentBinding(value, field);
  }

  private static void requireEntityRef(Object value, String field) {
    Map<String, Object> ref = requireObject(value, field);
    long kind = requiredLong(ref, "kind");
    if (kind < 1 || kind > 4) {
      throw SDKError.validation("runtime_receipt", field + ".kind is not canonical");
    }
    requiredString(ref, "ura");
    requiredString(ref, "profile");
  }

  private static Map<String, Object> requireAuthorityBinding(Object value, String field) {
    Map<String, Object> binding = requireObject(value, field);
    requiredString(binding, "kind");
    return binding;
  }

  private static void requireSignature(Object value, String field) {
    Map<String, Object> signature = requireObject(value, field);
    requiredString(signature, "algorithm");
    base64Bytes(
        requiredString(signature, "signature_base64"),
        field + ".signature_base64",
        0,
        false);
  }

  private static byte[] receiptHash(Map<String, Object> raw, String field, boolean allowZero) {
    String value = requiredString(raw, field);
    byte[] decoded;
    try {
      decoded = HEX.parseHex(value);
    } catch (IllegalArgumentException error) {
      throw SDKError.validation("runtime_receipt", field + " must be hexadecimal");
    }
    if (decoded.length != 32) {
      throw SDKError.validation("runtime_receipt", field + " must be exactly 32 bytes");
    }
    boolean allZero = true;
    for (byte current : decoded) {
      allZero = allZero && current == 0;
    }
    if (allZero && !allowZero) {
      throw SDKError.validation("runtime_receipt", field + " must not be all-zero");
    }
    return decoded;
  }

  private static byte[] base64Bytes(
      String value, String field, int expectedLength, boolean allowEmpty) {
    if (value == null || value.isBlank()) {
      if (allowEmpty) {
        return new byte[0];
      }
      throw SDKError.validation("runtime_receipt", field + " is required");
    }
    byte[] decoded;
    try {
      decoded = Base64.getDecoder().decode(value);
    } catch (IllegalArgumentException error) {
      throw SDKError.validation("runtime_receipt", field + " must be valid base64");
    }
    if (decoded.length == 0 && !allowEmpty) {
      throw SDKError.validation("runtime_receipt", field + " must decode to non-empty bytes");
    }
    if (expectedLength > 0 && decoded.length != expectedLength) {
      throw SDKError.validation(
          "runtime_receipt", field + " must decode to exactly " + expectedLength + " bytes");
    }
    return decoded;
  }

  private static Map<String, Object> requireObject(Object value, String field) {
    if (!(value instanceof Map<?, ?> raw)) {
      throw SDKError.validation("runtime_receipt", field + " must be an object");
    }
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : raw.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw SDKError.validation("runtime_receipt", field + " object keys must be strings");
      }
      out.put(key, entry.getValue());
    }
    return Map.copyOf(out);
  }

  private static List<Object> requireList(Object value, String field) {
    if (!(value instanceof List<?> raw)) {
      throw SDKError.validation("runtime_receipt", field + " must be an array");
    }
    return List.copyOf(raw);
  }

  private static String requiredString(Map<String, Object> raw, String field) {
    Object value = raw.get(field);
    if (!(value instanceof String string) || string.isBlank()) {
      throw SDKError.validation("runtime_receipt", "runtime receipt summary is missing " + field);
    }
    return string;
  }

  private static String optionalString(Object value) {
    return value instanceof String string ? string : "";
  }

  private static long requiredLong(Map<String, Object> raw, String field) {
    Object value = raw.get(field);
    if (value instanceof Long longValue) {
      return longValue;
    }
    if (value instanceof Integer integer) {
      return integer.longValue();
    }
    throw SDKError.validation("runtime_receipt", field + " must be an integer");
  }
}
