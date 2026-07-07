package run.easynet.daemon;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

final class SurfaceSupport {
  static final String PROFILE = "surface";
  static final int DEFAULT_PAGE_SIZE = 50;
  static final int MAX_PAGE_SIZE = 500;

  private SurfaceSupport() {}

  static String cleanRequired(String value, String name) {
    String cleaned = optionalClean(value, name);
    if (cleaned == null) {
      throw invalid(name + " is required");
    }
    return cleaned;
  }

  static String optionalClean(String value, String name) {
    if (value == null || value.isEmpty()) {
      return null;
    }
    if (!value.equals(value.trim())) {
      throw invalid(name + " must not contain surrounding whitespace");
    }
    return value;
  }

  static String projectID(String value) {
    String cleaned = cleanRequired(value, "project_id");
    if (!cleaned.matches("[A-Za-z0-9_-]{1,64}")) {
      throw invalid("invalid surface project_id");
    }
    return cleaned;
  }

  static String surfaceRef(String value) {
    String cleaned = cleanRequired(value, "surface_ref");
    if (cleaned.startsWith("http://") || cleaned.startsWith("https://")) {
      throw invalid("surface_ref must not be an HTTP route");
    }
    return cleaned;
  }

  static Map<String, Object> requiredObject(Map<String, Object> fields, String name) {
    Map<String, Object> value = optionalObject(fields.get(name), name);
    if (value == null) {
      throw invalid(name + " must be an object");
    }
    return value;
  }

  static Map<String, Object> optionalObject(Object value, String name) {
    if (value == null) {
      return null;
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

  static List<Object> requiredList(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (!(value instanceof List<?> list)) {
      throw invalid(name + " must be a list");
    }
    return List.copyOf(list);
  }

  static Map<String, Object> copyObject(Map<String, Object> value) {
    if (value == null || value.isEmpty()) {
      return Map.of();
    }
    return Collections.unmodifiableMap(new LinkedHashMap<>(value));
  }

  static String requiredString(Map<String, Object> fields, String name) {
    return cleanRequired(optionalString(fields.get(name), name), name);
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

  static int requiredInteger(Map<String, Object> fields, String name) {
    Integer value = optionalInteger(fields.get(name), name);
    if (value == null) {
      throw invalid(name + " must be a non-negative integer");
    }
    return value;
  }

  static Integer optionalInteger(Object value, String name) {
    if (value == null) {
      return null;
    }
    if (value instanceof Number number) {
      double doubleValue = number.doubleValue();
      long longValue = number.longValue();
      if (longValue >= 0 && longValue <= Integer.MAX_VALUE && doubleValue == (double) longValue) {
        return Math.toIntExact(longValue);
      }
    }
    throw invalid(name + " must be a non-negative integer or null");
  }

  static boolean requiredBoolean(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (value instanceof Boolean bool) {
      return bool;
    }
    throw invalid(name + " must be a boolean");
  }

  static void putOptional(Map<String, Object> out, String name, String value) {
    if (value != null) {
      out.put(name, value);
    }
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
