package run.runtime.sdk;

import java.util.LinkedHashMap;
import java.util.Map;

public record SignerPolicy(String mode, String signerId, String policyRef, long expiresAtUnixMS) {
  public SignerPolicy {
    mode = mode == null ? "" : mode;
    signerId = signerId == null ? "" : signerId;
    policyRef = policyRef == null ? "" : policyRef;
    if (expiresAtUnixMS < 0) {
      throw SDKError.validation("signer_policy", "expires_at_unix_ms must be non-negative");
    }
  }

  static SignerPolicy fromObject(Map<String, Object> fields) {
    rejectUnknown(fields, "mode", "signer_id", "policy_ref", "expires_at_unix_ms");
    return new SignerPolicy(
        optionalString(fields, "mode"),
        optionalString(fields, "signer_id"),
        optionalString(fields, "policy_ref"),
        optionalNonNegativeLong(fields, "expires_at_unix_ms"));
  }

  Map<String, Object> toObject() {
    Map<String, Object> out = new LinkedHashMap<>();
    out.put("mode", mode);
    out.put("signer_id", signerId);
    out.put("policy_ref", policyRef);
    out.put("expires_at_unix_ms", expiresAtUnixMS);
    return out;
  }

  private static void rejectUnknown(Map<String, Object> fields, String... allowed) {
    java.util.Set<String> allowedSet = java.util.Set.of(allowed);
    for (String key : fields.keySet()) {
      if (!allowedSet.contains(key)) {
        throw SDKError.validation("signer_policy", key + " is not supported");
      }
    }
  }

  private static String optionalString(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    if (value == null) {
      return "";
    }
    if (!(value instanceof String string)) {
      throw SDKError.validation("signer_policy", field + " must be a string");
    }
    return string;
  }

  private static long optionalNonNegativeLong(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    if (value == null) {
      return 0;
    }
    long number;
    if (value instanceof Long longValue) {
      number = longValue;
    } else if (value instanceof Integer integerValue) {
      number = integerValue.longValue();
    } else {
      throw SDKError.validation("signer_policy", field + " must be an integer");
    }
    if (number < 0) {
      throw SDKError.validation("signer_policy", field + " must be non-negative");
    }
    return number;
  }
}
