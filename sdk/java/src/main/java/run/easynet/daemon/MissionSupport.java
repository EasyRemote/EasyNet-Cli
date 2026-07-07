package run.easynet.daemon;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

final class MissionSupport {
  static final String PROFILE = "mission";
  static final int MAX_EVENTS_LIMIT = 1000;

  private MissionSupport() {}

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

  static String missionID(String value) {
    String cleaned = required(value, "mission_id");
    if (cleaned.contains("/") || cleaned.contains("\\") || cleaned.equals(".") || cleaned.equals("..")) {
      throw invalid("mission_id must be an opaque mission identifier");
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

  static Long optionalLongObject(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (value == null) {
      return null;
    }
    return requiredLongValue(value, name);
  }

  static long requiredLong(Map<String, Object> fields, String name) {
    return requiredLongValue(fields.get(name), name);
  }

  static int requiredInteger(Map<String, Object> fields, String name) {
    long value = requiredLong(fields, name);
    if (value > Integer.MAX_VALUE) {
      throw invalid(name + " must fit int range");
    }
    return Math.toIntExact(value);
  }

  static boolean requiredBoolean(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (value instanceof Boolean bool) {
      return bool;
    }
    throw invalid(name + " must be a boolean");
  }

  static void requireChildInvocationFact(
      String stepID,
      String requestID,
      String traceID,
      String ability,
      String invocationURA,
      String callerURA,
      String calleeURA,
      String subjectURA,
      Map<String, Object> receipt) {
    if (receipt == null || receipt.isEmpty()) {
      return;
    }
    if (empty(stepID)
        || empty(requestID)
        || empty(traceID)
        || empty(ability)
        || empty(invocationURA)
        || empty(callerURA)
        || empty(calleeURA)
        || empty(subjectURA)) {
      throw invalid("receipt-backed child invocation facts must be complete");
    }
  }

  static void requireChildReceiptFact(String receiptURA, String receiptHash) {
    if (empty(receiptURA) || empty(receiptHash)) {
      throw invalid("child receipt refs require receipt_ura and receipt_hash");
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

  private static long requiredLongValue(Object value, String name) {
    if (value instanceof Number number) {
      double doubleValue = number.doubleValue();
      long longValue = number.longValue();
      if (longValue >= 0 && doubleValue == (double) longValue) {
        return longValue;
      }
    }
    throw invalid(name + " must be a non-negative integer");
  }

  private static boolean empty(String value) {
    return value == null || value.isEmpty();
  }
}
