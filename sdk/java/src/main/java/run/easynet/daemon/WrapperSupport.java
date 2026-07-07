package run.easynet.daemon;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Map;

final class WrapperSupport {
  static final String PROFILE = "wrappers";

  private WrapperSupport() {}

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

  static Map<String, Object> copyObject(Map<String, Object> value) {
    if (value == null || value.isEmpty()) {
      return Map.of();
    }
    return Collections.unmodifiableMap(new LinkedHashMap<>(value));
  }

  static void validateKind(String profile, String kind, String expectedKind) {
    if (!PROFILE.equals(profile) || !expectedKind.equals(kind)) {
      throw invalid("invalid " + expectedKind + " projection");
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
