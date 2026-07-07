package run.easynet.daemon;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Map;

final class CompanionSupport {
  static final String PROFILE = "desktop_companion";

  private CompanionSupport() {}

  static String requiredString(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (value instanceof String string && !string.trim().isEmpty()) {
      return string;
    }
    throw invalid(name + " must be a non-empty string");
  }

  static String optionalString(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (value == null) {
      return "";
    }
    if (value instanceof String string) {
      return string;
    }
    throw invalid(name + " must be a string or null");
  }

  static boolean requiredBoolean(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (value instanceof Boolean bool) {
      return bool;
    }
    throw invalid(name + " must be a boolean");
  }

  static Long optionalLong(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (value == null) {
      return null;
    }
    if (value instanceof Number number) {
      double doubleValue = number.doubleValue();
      long longValue = number.longValue();
      if (longValue >= 0 && doubleValue == (double) longValue) {
        return longValue;
      }
    }
    throw invalid(name + " must be a non-negative integer or null");
  }

  static Map<String, Object> optionalObject(Map<String, Object> fields, String name) {
    return optionalObject(fields.get(name), name);
  }

  static Map<String, Object> nullableObject(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    return value == null ? null : optionalObject(value, name);
  }

  static Map<String, Object> optionalObject(Object value, String name) {
    if (value == null) {
      return Map.of();
    }
    if (!(value instanceof Map<?, ?> decoded)) {
      throw invalid(name + " must be an object or null");
    }
    LinkedHashMap<String, Object> copied = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : decoded.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw invalid(name + " keys must be strings");
      }
      copied.put(key, entry.getValue());
    }
    return Collections.unmodifiableMap(copied);
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

  static SDKError transport(String message, Throwable cause) {
    return new SDKError(
        ErrorCode.TRANSPORT,
        "transport",
        RetryHint.SAFE,
        true,
        message,
        "",
        "",
        "",
        Map.of("profile", PROFILE),
        cause);
  }
}
