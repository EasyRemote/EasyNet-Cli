package run.easynet.daemon;

import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

final class DirectoryIdentitySupport {
  private DirectoryIdentitySupport() {}

  static String cleanRequired(String value, String name) {
    String cleaned = optionalClean(value, name);
    if (cleaned == null) {
      throw invalidField(name, "is required");
    }
    return cleaned;
  }

  static String optionalClean(String value, String name) {
    if (value == null) {
      return null;
    }
    if (value.isBlank()) {
      throw invalidField(name, "must be non-empty");
    }
    if (!value.equals(value.trim())) {
      throw invalidField(name, "must not contain surrounding whitespace");
    }
    return value;
  }

  static String requiredString(Map<String, Object> fields, String name) {
    return cleanRequired(optionalString(fields.get(name), name), name);
  }

  static String requiredString(String value, String name) {
    return cleanRequired(value, name);
  }

  static boolean requiredBoolean(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (value instanceof Boolean bool) {
      return bool;
    }
    throw invalidField(name, "must be a boolean");
  }

  static int requiredInteger(Map<String, Object> fields, String name) {
    Integer value = optionalInteger(fields.get(name), name);
    if (value != null) {
      return value;
    }
    throw invalidField(name, "must be a non-negative integer");
  }

  static String optionalString(Object value, String name) {
    if (value == null) {
      return null;
    }
    if (value instanceof String string) {
      return string;
    }
    throw invalidField(name, "must be a string or null");
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
    throw invalidField(name, "must be a non-negative integer or null");
  }

  static Integer normalizeLimit(Integer limit) {
    int normalized = limit == null ? DirectoryClient.DEFAULT_DIRECTORY_PAGE_SIZE : limit;
    if (normalized < 1 || normalized > DirectoryClient.MAX_DIRECTORY_PAGE_SIZE) {
      throw invalidField("limit", "must be between 1 and 500");
    }
    return normalized;
  }

  static Map<String, Object> requiredObject(Map<String, Object> fields, String name) {
    return requiredCopiedObject(optionalObject(fields.get(name), name), name);
  }

  static Map<String, Object> requiredCopiedObject(Map<String, Object> value, String name) {
    if (value == null) {
      throw invalidField(name, "must be an object");
    }
    return copyObject(value);
  }

  static Map<String, Object> optionalObject(Object value, String name) {
    if (value == null) {
      return null;
    }
    if (!(value instanceof Map<?, ?> decoded)) {
      throw invalidField(name, "must be an object or null");
    }
    LinkedHashMap<String, Object> copied = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : decoded.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw invalidField(name, "keys must be strings");
      }
      copied.put(key, entry.getValue());
    }
    return Collections.unmodifiableMap(copied);
  }

  static Map<String, Object> copyObject(Map<String, Object> value) {
    if (value == null || value.isEmpty()) {
      return Map.of();
    }
    return Collections.unmodifiableMap(new LinkedHashMap<>(value));
  }

  static List<Object> requiredList(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (!(value instanceof List<?> list)) {
      throw invalidField(name, "must be an array");
    }
    return copyList(list);
  }

  static List<Object> copyList(List<?> value) {
    if (value == null || value.isEmpty()) {
      return List.of();
    }
    return Collections.unmodifiableList(new ArrayList<>(value));
  }

  static SDKError invalidField(String name, String message) {
    return new SDKError(
        ErrorCode.INVALID_ARGUMENT,
        "decode",
        RetryHint.NEVER,
        false,
        "directory_identity field " + name + " " + message,
        "",
        "",
        "",
        Map.of("field", name),
        null);
  }

  static SDKError notImplemented(String message) {
    return new SDKError(
        ErrorCode.NOT_IMPLEMENTED,
        "transport",
        RetryHint.NEVER,
        false,
        message,
        "",
        "",
        "",
        Map.of(),
        null);
  }
}
