package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

final class ReceiptSupport {
  static final String PROFILE = "receipt";
  static final String FETCH_ABILITY = "invocation.history.get";

  private ReceiptSupport() {}

  static String required(String value, String field) {
    if (value == null || value.isBlank() || !value.equals(value.trim())) {
      throw invalid(field + " is required");
    }
    return value;
  }

  static String optional(String value, String field) {
    if (value == null || value.isEmpty()) {
      return "";
    }
    if (!value.equals(value.trim())) {
      throw invalid(field + " must not contain surrounding whitespace");
    }
    return value;
  }

  static Map<String, Object> requiredObject(Map<String, Object> value, String field) {
    if (value == null || value.isEmpty()) {
      throw invalid(field + " is required");
    }
    return Map.copyOf(value);
  }

  static String optionalJSON(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    if (value == null) {
      return "";
    }
    if (value instanceof String string) {
      return string;
    }
    throw invalid(field + " must be a string or null");
  }

  static String requiredJSON(Map<String, Object> fields, String field) {
    String value = optionalJSON(fields, field);
    if (value.isBlank()) {
      throw invalid(field + " is required");
    }
    return value;
  }

  static boolean requiredBoolean(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    if (value instanceof Boolean bool) {
      return bool;
    }
    throw invalid(field + " must be a boolean");
  }

  static int optionalIndex(Map<String, Object> fields, String field) {
    Integer value = RuntimeHealth.optionalInteger(fields.get(field), field);
    return value == null ? -1 : value;
  }

  static Map<String, Object> optionalObject(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    if (value == null) {
      return Map.of();
    }
    if (!(value instanceof Map<?, ?> raw)) {
      throw invalid(field + " must be an object or null");
    }
    LinkedHashMap<String, Object> copied = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : raw.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw invalid(field + " keys must be strings");
      }
      copied.put(key, entry.getValue());
    }
    return java.util.Collections.unmodifiableMap(copied);
  }

  static String normalizeHash(String value, String field) {
    String hash = required(value, field);
    if (!hash.matches("[0-9a-f]{64}")) {
      throw invalid(field + " must be 64 lowercase hex characters");
    }
    return hash;
  }

  static SDKError invalid(String message) {
    return new SDKError(
        ErrorCode.INVALID_ARGUMENT,
        PROFILE,
        RetryHint.NEVER,
        false,
        message,
        "",
        "",
        "",
        Map.of("profile", PROFILE),
        null);
  }

  static SDKError unsupported(String message) {
    return new SDKError(
        ErrorCode.NOT_IMPLEMENTED,
        "transport",
        RetryHint.NEVER,
        false,
        message,
        "",
        "",
        "",
        Map.of("profile", PROFILE),
        null);
  }
}
