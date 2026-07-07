package run.easynet.daemon;

import java.nio.file.Path;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

final class PublicationSupport {
  static final String PROFILE = "publication";

  private PublicationSupport() {}

  static String required(String value, String name) {
    if (value == null || value.isEmpty() || !value.equals(value.trim())) {
      throw invalid(name + " is required");
    }
    return value;
  }

  static String optional(String value, String name) {
    if (value == null || value.isEmpty()) {
      return null;
    }
    if (!value.equals(value.trim())) {
      throw invalid(name + " must not contain surrounding whitespace");
    }
    return value;
  }

  static String absolutePath(String value) {
    String cleaned = required(value, "path");
    if (!Path.of(cleaned).isAbsolute()) {
      throw invalid("absolute resource path is required");
    }
    return cleaned;
  }

  static String capability(String value) {
    String cleaned = required(value, "capability");
    if (!List.of("list", "stat", "read", "write").contains(cleaned)) {
      throw invalid("invalid resource capability");
    }
    return cleaned;
  }

  static Map<String, Object> copyObject(Map<String, Object> value) {
    if (value == null || value.isEmpty()) {
      return Map.of();
    }
    return Collections.unmodifiableMap(new LinkedHashMap<>(value));
  }

  static Map<String, Object> requiredObject(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (!(value instanceof Map<?, ?> decoded)) {
      throw invalid(name + " must be an object");
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

  static List<Object> requiredList(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (!(value instanceof List<?> list)) {
      throw invalid(name + " must be a list");
    }
    return List.copyOf(list);
  }

  static String requiredString(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (value instanceof String string && !string.isEmpty()) {
      return string;
    }
    throw invalid(name + " must be a non-empty string");
  }

  static String optionalString(Object value, String name) {
    if (value == null) {
      return null;
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

  static long requiredLong(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (value instanceof Number number) {
      double doubleValue = number.doubleValue();
      long longValue = number.longValue();
      if (longValue >= 0 && doubleValue == (double) longValue) {
        return longValue;
      }
    }
    throw invalid(name + " must be a non-negative integer");
  }

  static Long optionalLong(Object value, String name) {
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
