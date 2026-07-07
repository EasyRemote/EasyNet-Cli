package run.easynet.daemon;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

final class CompatibilitySupport {
  static final String PROFILE = "compatibility";

  private CompatibilitySupport() {}

  static String requiredString(Object value, String field) {
    if (!(value instanceof String text) || text.isEmpty()) {
      throw invalid(field + " is required");
    }
    if (!text.equals(text.trim())) {
      throw invalid(field + " must not contain surrounding whitespace");
    }
    return text;
  }

  static String optionalString(Object value, String field) {
    if (value == null) {
      return null;
    }
    if (value instanceof String text) {
      if (!text.equals(text.trim())) {
        throw invalid(field + " must not contain surrounding whitespace");
      }
      return text;
    }
    throw invalid(field + " must be a string or null");
  }

  static String requiredURA(Object value, String field) {
    String text = requiredString(value, field);
    if (!text.startsWith("easynet:///r/")) {
      throw invalid(field + " must be a URA");
    }
    return text;
  }

  static String requiredAbilityURA(Object value, String field) {
    String text = requiredURA(value, field);
    if (!text.contains("/ability/")) {
      throw invalid(field + " must be an Ability URA");
    }
    return text;
  }

  static Long optionalNonNegativeInteger(Object value, String field) {
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
    throw invalid(field + " must be a non-negative integer");
  }

  static long requiredNonNegativeInteger(Object value, String field) {
    Long integer = optionalNonNegativeInteger(value, field);
    if (integer == null) {
      throw invalid(field + " must be a non-negative integer");
    }
    return integer;
  }

  static boolean requiredTrue(Object value, String field) {
    if (Boolean.TRUE.equals(value)) {
      return true;
    }
    throw invalid(field + " must be true");
  }

  static Map<String, Object> requiredObject(Object value, String field) {
    if (!(value instanceof Map<?, ?> decoded)) {
      throw invalid(field + " must be an object");
    }
    LinkedHashMap<String, Object> copy = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : decoded.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw invalid(field + " keys must be strings");
      }
      copy.put(key, entry.getValue());
    }
    return Collections.unmodifiableMap(copy);
  }

  static Map<String, Object> optionalObject(Object value, String field) {
    if (value == null) {
      return Map.of();
    }
    return requiredObject(value, field);
  }

  static List<Object> requiredList(Object value, String field) {
    if (!(value instanceof List<?> list)) {
      throw invalid(field + " must be an array");
    }
    return List.copyOf(list);
  }

  static java.util.List<Map<String, Object>> requiredObjectList(Object value, String field) {
    java.util.ArrayList<Map<String, Object>> out = new java.util.ArrayList<>();
    for (Object item : requiredList(value, field)) {
      out.add(requiredObject(item, field + " item"));
    }
    return java.util.List.copyOf(out);
  }

  static Map<String, Object> copyObject(Map<String, Object> value) {
    if (value == null || value.isEmpty()) {
      return Map.of();
    }
    return Collections.unmodifiableMap(new LinkedHashMap<>(value));
  }

  static void validateHash(String value, String field) {
    String text = requiredString(value, field);
    if (!text.matches("sha256:[0-9a-f]{64}")) {
      throw invalid(field + " must use sha256:<64 lowercase hex> form");
    }
  }

  static void validateKind(String profile, String kind, String expectedKind) {
    if (!PROFILE.equals(profile) || !expectedKind.equals(kind)) {
      throw invalid("invalid " + expectedKind + " projection");
    }
  }

  static void validateObject(String object, String expected, String label) {
    if (!expected.equals(object)) {
      throw invalid("invalid " + label + " projection");
    }
  }

  static void putOptional(Map<String, Object> out, String key, Object value) {
    if (value != null) {
      out.put(key, value);
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
