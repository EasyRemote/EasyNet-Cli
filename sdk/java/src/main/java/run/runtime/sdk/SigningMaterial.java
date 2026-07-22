package run.runtime.sdk;

import java.util.LinkedHashMap;
import java.util.Map;

public record SigningMaterial(
    String algorithm,
    String canonicalBytesBase64,
    String argsDigestHex,
    String descriptorRef,
    long expiresAtUnixMS) {
  public SigningMaterial {
    algorithm = required(algorithm, "algorithm");
    canonicalBytesBase64 = required(canonicalBytesBase64, "canonical_bytes_base64");
    argsDigestHex = required(argsDigestHex, "args_digest_hex");
    descriptorRef = required(descriptorRef, "descriptor_ref");
    if (expiresAtUnixMS < 0) {
      throw SDKError.validation("signing_material", "expires_at_unix_ms must be non-negative");
    }
  }

  static SigningMaterial fromObject(Map<String, Object> fields) {
    rejectUnknown(
        fields,
        "algorithm",
        "canonical_bytes_base64",
        "args_digest_hex",
        "descriptor_ref",
        "expires_at_unix_ms");
    return new SigningMaterial(
        string(fields, "algorithm"),
        string(fields, "canonical_bytes_base64"),
        string(fields, "args_digest_hex"),
        string(fields, "descriptor_ref"),
        nonNegativeLong(fields, "expires_at_unix_ms"));
  }

  private static void rejectUnknown(Map<String, Object> fields, String... allowed) {
    java.util.Set<String> allowedSet = java.util.Set.of(allowed);
    for (String key : fields.keySet()) {
      if (!allowedSet.contains(key)) {
        throw SDKError.validation("signing_material", key + " is not supported");
      }
    }
  }

  Map<String, Object> toObject() {
    Map<String, Object> out = new LinkedHashMap<>();
    out.put("algorithm", algorithm);
    out.put("canonical_bytes_base64", canonicalBytesBase64);
    out.put("args_digest_hex", argsDigestHex);
    out.put("descriptor_ref", descriptorRef);
    out.put("expires_at_unix_ms", expiresAtUnixMS);
    return out;
  }

  private static String string(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    if (!(value instanceof String string) || string.isBlank()) {
      throw SDKError.validation("signing_material", field + " is required");
    }
    return string;
  }

  private static long nonNegativeLong(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    long number;
    if (value instanceof Long longValue) {
      number = longValue;
    } else if (value instanceof Integer integerValue) {
      number = integerValue.longValue();
    } else {
      throw SDKError.validation("signing_material", field + " must be an integer");
    }
    if (number < 0) {
      throw SDKError.validation("signing_material", field + " must be non-negative");
    }
    return number;
  }

  private static String required(String value, String field) {
    if (value == null || value.isBlank()) {
      throw SDKError.validation("signing_material", field + " is required");
    }
    return value;
  }
}
