package run.easynet.daemon;

import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

final class AdminSupport {
  static final String PROFILE = "admin_gateway";

  private AdminSupport() {}

  static String required(String value, String name) {
    if (value == null || value.isEmpty() || !value.equals(value.trim())) {
      throw invalid(name + " is required");
    }
    return value;
  }

  static String optional(String value, String name) {
    if (value == null || value.isEmpty()) {
      return "";
    }
    if (!value.equals(value.trim())) {
      throw invalid(name + " must not contain surrounding whitespace");
    }
    return value;
  }

  static String identifier(String value, String name) {
    String cleaned = required(value, name);
    if (cleaned.contains("/") || cleaned.contains("\\") || cleaned.chars().anyMatch(ch -> Character.isWhitespace(ch))) {
      throw invalid(name + " must be an opaque daemon identifier");
    }
    return cleaned;
  }

  static String agentName(String value) {
    String cleaned = identifier(value, "name");
    if (cleaned.equals("device") || cleaned.startsWith("device.")) {
      throw invalid("device system agents are not managed by hosted agent lifecycle");
    }
    return cleaned;
  }

  static String optionalAgentName(String value) {
    return value == null || value.isEmpty() ? "" : agentName(value);
  }

  static String agentURA(String value) {
    String cleaned = required(value, "agent_ura");
    if (!cleaned.contains("/agent/")) {
      throw invalid("agent_ura must be an Agent URA");
    }
    if (cleaned.contains("/agent/device.")) {
      throw invalid("device-sponsored System Agents are not managed by hosted agent lifecycle");
    }
    return cleaned;
  }

  static String optionalAgentURA(String value) {
    return value == null || value.isEmpty() ? "" : agentURA(value);
  }

  static String hubURA(String value) {
    String cleaned = required(value, "hub_ura");
    if (!cleaned.contains("/hub/")) {
      throw invalid("hub_ura must be a Hub URA");
    }
    return cleaned;
  }

  static String deviceURA(String value) {
    String cleaned = required(value, "device_ura");
    if (!cleaned.contains("/device/")) {
      throw invalid("device_ura must be a Device URA");
    }
    return cleaned;
  }

  static String reason(String value) {
    String cleaned = required(value, "reason");
    if (cleaned.chars().anyMatch(ch -> ch < 0x20 || ch == 0x7f)) {
      throw invalid("reason must not contain control characters");
    }
    return cleaned;
  }

  static String optionalReason(String value) {
    return value == null || value.isEmpty() ? "" : reason(value);
  }

  static List<String> scopes(List<String> values) {
    if (values == null || values.isEmpty()) {
      return List.of();
    }
    ArrayList<String> out = new ArrayList<>();
    for (String value : values) {
      out.add(identifier(value, "scope"));
    }
    return List.copyOf(out);
  }

  static Map<String, Object> copyObject(Map<String, Object> value) {
    if (value == null || value.isEmpty()) {
      return Map.of();
    }
    return Collections.unmodifiableMap(new LinkedHashMap<>(value));
  }

  static List<Object> copyList(List<?> value) {
    if (value == null || value.isEmpty()) {
      return List.of();
    }
    return List.copyOf(value);
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

  static Map<String, Object> requiredObject(Map<String, Object> fields, String name) {
    Map<String, Object> value = optionalObject(fields.get(name), name);
    if (value == null) {
      throw invalid(name + " must be an object");
    }
    return value;
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

  static Boolean optionalBoolean(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (value == null) {
      return null;
    }
    if (value instanceof Boolean bool) {
      return bool;
    }
    throw invalid(name + " must be a boolean or null");
  }

  static long requiredLong(Map<String, Object> fields, String name) {
    return longValue(fields.get(name), name);
  }

  static Long optionalLong(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    return value == null ? null : longValue(value, name);
  }

  static void putOptional(Map<String, Object> out, String name, String value) {
    if (value != null && !value.isEmpty()) {
      out.put(name, value);
    }
  }

  static void putOptional(Map<String, Object> out, String name, Object value) {
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

  private static long longValue(Object value, String name) {
    if (value instanceof Number number) {
      double doubleValue = number.doubleValue();
      long longValue = number.longValue();
      if (longValue >= 0 && doubleValue == (double) longValue) {
        return longValue;
      }
    }
    throw invalid(name + " must be a non-negative integer");
  }
}
