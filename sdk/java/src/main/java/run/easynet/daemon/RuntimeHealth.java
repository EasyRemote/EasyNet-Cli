package run.easynet.daemon;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Objects;

public record RuntimeHealth(
    boolean apiReady,
    boolean daemonReady,
    boolean invocationReady,
    boolean directoryReady,
    boolean trustReady,
    boolean runtimeReady,
    String version,
    Integer abiVersion,
    Map<String, Object> mismatch,
    List<String> diagnostics) {
  public RuntimeHealth {
    mismatch =
        mismatch == null
            ? null
            : java.util.Collections.unmodifiableMap(new java.util.LinkedHashMap<>(mismatch));
    diagnostics = diagnostics == null ? List.of() : List.copyOf(diagnostics);
  }

  public static RuntimeHealth fromJSON(byte[] raw) {
    Objects.requireNonNull(raw, "raw");
    Map<String, Object> fields = JsonValueReader.object(raw, "runtime health JSON");
    return new RuntimeHealth(
        requiredBoolean(fields, "api_ready"),
        requiredBoolean(fields, "daemon_ready"),
        requiredBoolean(fields, "invocation_ready"),
        requiredBoolean(fields, "directory_ready"),
        requiredBoolean(fields, "trust_ready"),
        requiredBoolean(fields, "runtime_ready"),
        optionalString(fields.get("version"), "version"),
        optionalInteger(fields.get("abi_version"), "abi_version"),
        optionalObject(fields.get("mismatch"), "mismatch"),
        diagnostics(fields.get("diagnostics")));
  }

  public boolean apiAlive() {
    return apiReady && daemonReady;
  }

  public boolean ready() {
    return runtimeReady;
  }

  static boolean requiredBoolean(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (value instanceof Boolean bool) {
      return bool;
    }
    throw invalidField(name, "must be a boolean");
  }

  static String requiredString(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (value instanceof String string && !string.isBlank()) {
      return string;
    }
    throw invalidField(name, "must be a non-empty string");
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

  static Map<String, Object> optionalObject(Object value, String name) {
    if (value == null) {
      return null;
    }
    if (!(value instanceof Map<?, ?> decoded)) {
      throw invalidField(name, "must be an object or null");
    }
    return copyStringKeyedMap(decoded, name);
  }

  static List<String> diagnostics(Object value) {
    if (value == null) {
      return List.of();
    }
    if (!(value instanceof List<?> values)) {
      throw invalidField("diagnostics", "must be an array");
    }
    List<String> diagnostics = new ArrayList<>();
    for (Object item : values) {
      if (!(item instanceof String diagnostic)) {
        throw invalidField("diagnostics", "items must be strings");
      }
      diagnostics.add(diagnostic);
    }
    return diagnostics;
  }

  private static Map<String, Object> copyStringKeyedMap(Map<?, ?> decoded, String name) {
    java.util.LinkedHashMap<String, Object> copied = new java.util.LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : decoded.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw invalidField(name, "keys must be strings");
      }
      copied.put(key, entry.getValue());
    }
    return java.util.Collections.unmodifiableMap(copied);
  }

  static SDKError invalidField(String name, String message) {
    return new SDKError(
        ErrorCode.INVALID_ARGUMENT,
        "decode",
        RetryHint.NEVER,
        false,
        "runtime health field " + name + " " + message,
        "",
        "",
        "",
        Map.of("field", name),
        null);
  }
}
