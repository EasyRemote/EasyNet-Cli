package run.runtime.sdk;

import java.io.ByteArrayOutputStream;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.Arrays;
import java.util.Base64;
import java.util.HexFormat;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

final class RuntimeReceiptProofFacts {
  private static final HexFormat HEX = HexFormat.of();

  private RuntimeReceiptProofFacts() {}

  static void validate(Map<String, Object> raw) {
    Map<String, Object> callerBinding = requireAgentBinding(raw.get("caller_binding"), "caller_binding");
    Map<String, Object> calleeBinding = requireAgentBinding(raw.get("callee_binding"), "callee_binding");
    requireAgentBinding(raw.get("subject_binding"), "subject_binding");
    base64Bytes(
        requiredString(raw, "invocation_nonce_base64"),
        "invocation_nonce_base64",
        16,
        false);

    String causalKind = requiredString(raw, "causal_binding_kind");
    Map<String, Object> causalBinding = requireObject(raw.get("causal_binding"), "causal_binding");
    validateCausalBinding(causalKind, causalBinding);

    requireSignature(raw.get("callee_signature"), "callee_signature");
    Map<String, Object> signerBinding = requireAgentBinding(raw.get("signer_binding"), "signer_binding");
    validateSigningModel(calleeBinding, signerBinding, optionalString(raw.get("host_attestation_base64")));

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
    requireExactKeys(
        authorityProof,
        "authority_proof",
        "proof_type",
        "binding_kind",
        "binding",
        "proof_payload_base64",
        "proof_hash_hex",
        "issuer",
        "signature",
        "admission_hook");
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

    byte[] proofPayload =
        base64Bytes(
            requiredStringAllowEmpty(authorityProof, "proof_payload_base64"),
            "authority_proof.proof_payload_base64",
            0,
            true);
    byte[] proofHash = receiptHash(authorityProof, "proof_hash_hex", false);
    validateAuthorityProofHash(proofPayload, proofBinding, proofHash);
    Map<String, Object> issuer = requireAgentBinding(authorityProof.get("issuer"), "authority_proof.issuer");
    requireSameIdentity(issuer, calleeBinding);
    if (authorityProof.containsKey("signature") && authorityProof.get("signature") != null) {
      requireSignature(authorityProof.get("signature"), "authority_proof.signature");
    }
    requiredString(authorityProof, "admission_hook");

    receiptHash(raw, "input_hash_hex", false);
    receiptHash(raw, "output_hash_hex", false);
    requireParentReceipts(raw.get("parent_receipts"));
  }

  static byte[] receiptHash(Map<String, Object> raw, String field, boolean allowZero) {
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
    if (isZeroHash(decoded) && !allowZero) {
      throw SDKError.validation("runtime_receipt", field + " must not be all-zero");
    }
    return decoded;
  }

  private static void validateSigningModel(
      Map<String, Object> calleeBinding, Map<String, Object> signerBinding, String hostAttestation) {
    String calleeURA = requiredString(calleeBinding, "ura");
    String signerURA = requiredString(signerBinding, "ura");
    if (signerURA.equals(calleeURA)) {
      if (!hostAttestation.isBlank()) {
        throw SDKError.validation(
            "runtime_receipt",
            "self-signed runtime receipt must not carry host_attestation_base64");
      }
      return;
    }
    if (hostAttestation.isBlank()) {
      throw SDKError.validation(
          "runtime_receipt", "hosted runtime receipt is missing host_attestation_base64");
    }
    base64Bytes(hostAttestation, "host_attestation_base64", 64, false);
  }

  private static void validateAuthorityProofHash(
      byte[] proofPayload, Map<String, Object> proofBinding, byte[] proofHash) {
    byte[] expected =
        proofPayload.length > 0
            ? sha256(proofPayload)
            : sha256(canonicalAuthorityBytes(proofBinding, "authority_proof.binding"));
    if (isZeroHash(expected) || isZeroHash(proofHash) || !Arrays.equals(proofHash, expected)) {
      throw SDKError.validation(
          "runtime_receipt", "runtime receipt proof facts are not canonical: authority_proof_hash_mismatch");
    }
  }

  private static byte[] canonicalAuthorityBytes(Map<String, Object> binding, String field) {
    ByteArrayOutputStream out = new ByteArrayOutputStream();
    String kind = requiredString(binding, "kind");
    switch (kind) {
      case "self" -> {
        out.write(0x01);
        putString(out, requiredString(binding, "principal_ura"));
      }
      case "delegation" -> {
        out.write(0x02);
        putString(out, requiredString(binding, "issuer_ura"));
        putString(out, requiredString(binding, "subject_ura"));
        putString(out, requiredString(binding, "caller_ura"));
        putString(out, requiredString(binding, "audience"));
        List<String> scopes = requiredStringList(binding.get("scopes"), field + ".scopes");
        putU32(out, scopes.size());
        for (String scope : scopes) {
          putString(out, scope);
        }
        putI64(out, requiredNonNegativeLong(binding.get("issued_at_ms"), field + ".issued_at_ms"));
        putI64(out, requiredNonNegativeLong(binding.get("expires_at_ms"), field + ".expires_at_ms"));
        putBytes(
            out,
            base64Bytes(
                requiredString(binding, "signature_base64"),
                field + ".signature_base64",
                64,
                false));
      }
      case "capability" -> {
        out.write(0x03);
        putString(out, requiredString(binding, "capability_ura"));
      }
      case "policy" -> {
        out.write(0x04);
        putString(out, requiredString(binding, "policy_ura"));
      }
      case "session" -> {
        out.write(0x05);
        putString(out, requiredString(binding, "issuer_ura"));
        putString(out, requiredString(binding, "subject_ura"));
        putString(out, requiredString(binding, "session_id"));
        List<String> scopes = requiredStringList(binding.get("scopes"), field + ".scopes");
        putU32(out, scopes.size());
        for (String scope : scopes) {
          putString(out, scope);
        }
        List<String> audiences = requiredStringList(binding.get("audiences"), field + ".audiences");
        putU32(out, audiences.size());
        for (String audience : audiences) {
          putString(out, audience);
        }
        putI64(out, requiredNonNegativeLong(binding.get("issued_at_ms"), field + ".issued_at_ms"));
        putI64(out, requiredNonNegativeLong(binding.get("expires_at_ms"), field + ".expires_at_ms"));
        putBytes(
            out,
            base64Bytes(
                requiredString(binding, "signature_base64"),
                field + ".signature_base64",
                64,
                false));
      }
      case "bootstrap" -> {
        out.write(0x06);
        putString(out, requiredString(binding, "principal_ura"));
        putString(out, requiredString(binding, "realm"));
        putString(out, requiredString(binding, "ability"));
      }
      default -> throw SDKError.validation("runtime_receipt", field + ".kind is not canonical: " + kind);
    }
    return out.toByteArray();
  }

  private static void validateCausalBinding(String kind, Map<String, Object> binding) {
    String form = requiredString(binding, "form");
    if (!form.equals(kind)) {
      throw SDKError.validation(
          "runtime_receipt",
          "runtime receipt causal_binding form does not match causal_binding_kind");
    }
    switch (form) {
      case "none" -> requireExactKeys(binding, "causal_binding", "form");
      case "scalar" -> {
        requireExactKeys(binding, "causal_binding", "form", "receipt");
        requireReceiptRef(binding.get("receipt"), "causal_binding.receipt");
      }
      case "list" -> {
        requireExactKeys(binding, "causal_binding", "form", "prior");
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
        requireExactKeys(binding, "causal_binding", "form", "root_hex", "proof_ura");
        receiptHash(binding, "root_hex", false);
        requiredString(binding, "proof_ura");
      }
      default ->
          throw SDKError.validation("runtime_receipt", "unsupported causal_binding form " + form);
    }
  }

  private static void requireReceiptRef(Object value, String field) {
    Map<String, Object> ref = requireObject(value, field);
    requireExactKeys(ref, field, "receipt_hash_hex", "receipt_ura");
    receiptHash(ref, "receipt_hash_hex", false);
    requiredString(ref, "receipt_ura");
  }

  private static void requireParentReceipts(Object value) {
    List<Object> parents = requireList(value, "parent_receipts");
    for (int i = 0; i < parents.size(); i++) {
      requireReceiptRef(parents.get(i), "parent_receipts[" + i + "]");
    }
  }

  private static Map<String, Object> requireAgentBinding(Object value, String field) {
    Map<String, Object> binding = requireObject(value, field);
    requireExactKeys(binding, field, "ura", "profile");
    requiredString(binding, "ura");
    validateUraProfile(requiredString(binding, "profile"), field + ".profile");
    return binding;
  }

  private static void requireSameIdentity(
      Map<String, Object> issuer, Map<String, Object> calleeBinding) {
    String issuerURA = requiredString(issuer, "ura");
    String issuerProfile = requiredString(issuer, "profile");
    String calleeURA = requiredString(calleeBinding, "ura");
    String calleeProfile = requiredString(calleeBinding, "profile");
    if (!issuerURA.equals(calleeURA) || !issuerProfile.equals(calleeProfile)) {
      throw SDKError.validation(
          "runtime_receipt", "runtime receipt authority_proof issuer does not match callee_binding");
    }
  }

  private static void requireEntityRef(Object value, String field) {
    Map<String, Object> ref = requireObject(value, field);
    requireExactKeys(ref, field, "kind", "ura", "profile");
    long kind = requiredLong(ref, "kind");
    if (kind < 1 || kind > 7) {
      throw SDKError.validation("runtime_receipt", field + ".kind is not canonical");
    }
    requiredString(ref, "ura");
    validateUraProfile(requiredString(ref, "profile"), field + ".profile");
  }

  private static Map<String, Object> requireAuthorityBinding(Object value, String field) {
    Map<String, Object> binding = requireObject(value, field);
    switch (requiredString(binding, "kind")) {
      case "self" -> requireExactKeys(binding, field, "kind", "principal_ura");
      case "delegation" ->
          requireExactKeys(
              binding,
              field,
              "kind",
              "issuer_ura",
              "subject_ura",
              "caller_ura",
              "audience",
              "scopes",
              "issued_at_ms",
              "expires_at_ms",
              "signature_base64");
      case "capability" -> requireExactKeys(binding, field, "kind", "capability_ura");
      case "policy" -> requireExactKeys(binding, field, "kind", "policy_ura");
      case "session" ->
          requireExactKeys(
              binding,
              field,
              "kind",
              "issuer_ura",
              "subject_ura",
              "session_id",
              "scopes",
              "audiences",
              "issued_at_ms",
              "expires_at_ms",
              "signature_base64");
      case "bootstrap" -> requireExactKeys(binding, field, "kind", "principal_ura", "realm", "ability");
      default ->
          throw SDKError.validation(
              "runtime_receipt", field + ".kind is not canonical: " + binding.get("kind"));
    }
    return binding;
  }

  private static void requireSignature(Object value, String field) {
    Map<String, Object> signature = requireObject(value, field);
    requireExactKeys(signature, field, "algorithm", "signature_base64");
    requiredString(signature, "algorithm");
    base64Bytes(
        requiredString(signature, "signature_base64"),
        field + ".signature_base64",
        0,
        false);
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

  static Map<String, Object> requireObject(Object value, String field) {
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

  private static void requireExactKeys(Map<String, Object> object, String field, String... keys) {
    Set<String> allowed = Set.of(keys);
    for (String key : object.keySet()) {
      if (!allowed.contains(key)) {
        throw SDKError.validation(
            "runtime_receipt", field + " contains noncanonical field " + key);
      }
    }
  }

  private static List<Object> requireList(Object value, String field) {
    if (!(value instanceof List<?> raw)) {
      throw SDKError.validation("runtime_receipt", field + " must be an array");
    }
    return List.copyOf(raw);
  }

  private static List<String> requiredStringList(Object value, String field) {
    List<Object> raw = requireList(value, field);
    if (raw.isEmpty()) {
      throw SDKError.validation("runtime_receipt", field + " must be a non-empty array");
    }
    return raw.stream().map(item -> requiredListString(item, field)).toList();
  }

  private static String requiredListString(Object value, String field) {
    if (!(value instanceof String string) || string.trim().isEmpty()) {
      throw SDKError.validation("runtime_receipt", field + "[] must be a non-empty string");
    }
    return string.trim();
  }

  private static String requiredString(Map<String, Object> raw, String field) {
    Object value = raw.get(field);
    if (!(value instanceof String string) || string.trim().isEmpty()) {
      throw SDKError.validation("runtime_receipt", "runtime receipt summary is missing " + field);
    }
    return string.trim();
  }

  private static String requiredStringAllowEmpty(Map<String, Object> raw, String field) {
    Object value = raw.get(field);
    if (!(value instanceof String string) || !string.equals(string.trim())) {
      throw SDKError.validation("runtime_receipt", "runtime receipt summary is missing " + field);
    }
    return string;
  }

  private static String optionalString(Object value) {
    return value instanceof String string ? string.trim() : "";
  }

  private static long requiredLong(Map<String, Object> raw, String field) {
    return requiredNonNegativeLong(raw.get(field), field);
  }

  private static long requiredNonNegativeLong(Object value, String field) {
    long out;
    if (value instanceof Long longValue) {
      out = longValue;
    } else if (value instanceof Integer integer) {
      out = integer.longValue();
    } else if (value instanceof Short shortValue) {
      out = shortValue.longValue();
    } else if (value instanceof Byte byteValue) {
      out = byteValue.longValue();
    } else {
      throw SDKError.validation("runtime_receipt", field + " must be a non-negative integer");
    }
    if (out < 0) {
      throw SDKError.validation("runtime_receipt", field + " must be a non-negative integer");
    }
    return out;
  }

  private static void validateUraProfile(String profile, String field) {
    switch (profile) {
      case "axon-strict-v2" -> {}
      default -> throw SDKError.validation("runtime_receipt", field + " is not canonical");
    }
  }

  private static void putString(ByteArrayOutputStream out, String value) {
    putBytes(out, value.getBytes(StandardCharsets.UTF_8));
  }

  private static void putBytes(ByteArrayOutputStream out, byte[] value) {
    putU32(out, value.length);
    out.writeBytes(value);
  }

  private static void putU32(ByteArrayOutputStream out, int value) {
    out.writeBytes(ByteBuffer.allocate(4).putInt(value).array());
  }

  private static void putI64(ByteArrayOutputStream out, long value) {
    out.writeBytes(ByteBuffer.allocate(8).putLong(value).array());
  }

  private static byte[] sha256(byte[] bytes) {
    try {
      return MessageDigest.getInstance("SHA-256").digest(bytes);
    } catch (NoSuchAlgorithmException error) {
      throw SDKError.validation("runtime_receipt", "SHA-256 is unavailable");
    }
  }

  private static boolean isZeroHash(byte[] bytes) {
    for (byte value : bytes) {
      if (value != 0) {
        return false;
      }
    }
    return true;
  }
}
