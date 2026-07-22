package run.runtime.sdk;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Objects;

public record DiagnosticsReport(
    String profile,
    String kind,
    String state,
    boolean ready,
    String version,
    int abiVersion,
    String controlEndpoint,
    String invocationEndpoint,
    List<DiagnosticCheck> checks,
    List<String> diagnostics) {
  public DiagnosticsReport {
    if (!"health".equals(profile)) {
      throw RuntimeHealth.invalidField("profile", "must be health");
    }
    if (!"diagnostics_report".equals(kind)) {
      throw RuntimeHealth.invalidField("kind", "must be diagnostics_report");
    }
    checks = checks == null ? List.of() : List.copyOf(checks);
    diagnostics = diagnostics == null ? List.of() : List.copyOf(diagnostics);
    if (checks.isEmpty()) {
      throw RuntimeHealth.invalidField("checks", "must be non-empty");
    }
  }

  public static DiagnosticsReport fromJSON(byte[] raw) {
    Objects.requireNonNull(raw, "raw");
    Map<String, Object> fields = JsonValueReader.object(raw, "diagnostics JSON");
    return new DiagnosticsReport(
        RuntimeHealth.requiredString(fields, "profile"),
        RuntimeHealth.requiredString(fields, "kind"),
        RuntimeHealth.requiredString(fields, "state"),
        RuntimeHealth.requiredBoolean(fields, "ready"),
        RuntimeHealth.requiredString(fields, "version"),
        RuntimeHealth.requiredInteger(fields, "abi_version"),
        RuntimeHealth.requiredString(fields, "control_endpoint"),
        RuntimeHealth.optionalString(fields.get("invocation_endpoint"), "invocation_endpoint"),
        checks(fields.get("checks")),
        RuntimeHealth.diagnostics(fields.get("diagnostics")));
  }

  private static List<DiagnosticCheck> checks(Object value) {
    if (!(value instanceof List<?> values) || values.isEmpty()) {
      throw RuntimeHealth.invalidField("checks", "must be non-empty");
    }
    List<DiagnosticCheck> checks = new ArrayList<>();
    for (Object item : values) {
      if (!(item instanceof Map<?, ?> rawCheck)) {
        throw RuntimeHealth.invalidField("checks", "items must be objects");
      }
      Map<String, Object> check = copyCheck(rawCheck);
      checks.add(
          new DiagnosticCheck(
              RuntimeHealth.requiredString(check, "name"),
              RuntimeHealth.requiredBoolean(check, "ready"),
              RuntimeHealth.optionalString(check.get("message"), "message")));
    }
    return checks;
  }

  private static Map<String, Object> copyCheck(Map<?, ?> rawCheck) {
    java.util.LinkedHashMap<String, Object> copied = new java.util.LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : rawCheck.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw RuntimeHealth.invalidField("checks", "keys must be strings");
      }
      copied.put(key, entry.getValue());
    }
    return java.util.Collections.unmodifiableMap(copied);
  }
}
