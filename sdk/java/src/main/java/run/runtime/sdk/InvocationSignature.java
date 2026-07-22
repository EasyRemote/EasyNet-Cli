package run.runtime.sdk;

import java.util.LinkedHashMap;
import java.util.Map;

public record InvocationSignature(
    String algorithm, String signatureBase64, String keyIdHint, String signerPublicKeyBase64) {
  public InvocationSignature {
    algorithm = required(algorithm, "algorithm");
    signatureBase64 = required(signatureBase64, "signature_base64");
    keyIdHint = keyIdHint == null ? "" : keyIdHint;
    signerPublicKeyBase64 = signerPublicKeyBase64 == null ? "" : signerPublicKeyBase64;
  }

  static InvocationSignature fromObject(Map<String, Object> fields) {
    rejectUnknown(fields, "algorithm", "signature_base64", "key_id_hint", "signer_public_key_base64");
    return new InvocationSignature(
        string(fields, "algorithm"),
        string(fields, "signature_base64"),
        optionalString(fields, "key_id_hint"),
        optionalString(fields, "signer_public_key_base64"));
  }

  private static void rejectUnknown(Map<String, Object> fields, String... allowed) {
    java.util.Set<String> allowedSet = java.util.Set.of(allowed);
    for (String key : fields.keySet()) {
      if (!allowedSet.contains(key)) {
        throw SDKError.validation("signature", key + " is not supported");
      }
    }
  }

  Map<String, Object> toObject() {
    Map<String, Object> out = new LinkedHashMap<>();
    out.put("algorithm", algorithm);
    out.put("signature_base64", signatureBase64);
    out.put("key_id_hint", keyIdHint);
    out.put("signer_public_key_base64", signerPublicKeyBase64);
    return out;
  }

  private static String string(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    if (!(value instanceof String string) || string.isBlank()) {
      throw SDKError.validation("signature", field + " is required");
    }
    return string;
  }

  private static String optionalString(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    if (value == null) {
      return "";
    }
    if (!(value instanceof String string)) {
      throw SDKError.validation("signature", field + " must be a string");
    }
    return string;
  }

  private static String required(String value, String field) {
    if (value == null || value.isBlank()) {
      throw SDKError.validation("signature", field + " is required");
    }
    return value;
  }
}
