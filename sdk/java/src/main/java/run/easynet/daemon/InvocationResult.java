package run.easynet.daemon;

import java.util.Map;

public record InvocationResult(
    boolean ok,
    InvocationTerminalState terminalState,
    String outputJson,
    SDKError error,
    Map<String, Object> terminalReceipt) {
  public InvocationResult {
    if (terminalState == null) {
      throw SDKError.validation("runtime", "terminalState is required");
    }
    terminalReceipt = terminalReceipt == null ? Map.of() : Map.copyOf(terminalReceipt);
    if (ok && error != null) {
      throw SDKError.validation("runtime", "ok result must not carry error");
    }
    if (!ok && error == null) {
      throw SDKError.validation("runtime", "failed result must carry error");
    }
  }

  static InvocationResult fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "invocation result");
    boolean ok = bool(fields, "ok");
    String state = string(fields, "terminal_state");
    Object output = fields.get("output_json");
    Object terminalReceiptValue = fields.get("terminal_receipt");
    Map<String, Object> terminalReceipt =
        terminalReceiptValue instanceof Map<?, ?> map ? copyStringMap(map) : Map.of();
    SDKError error = ok ? null : SDKError.validation("runtime", "invocation failed");
    return new InvocationResult(
        ok,
        InvocationTerminalState.valueOf(camelEnum(state)),
        output == null ? "" : JsonValueWriter.write(output),
        error,
        terminalReceipt);
  }

  private static boolean bool(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    if (!(value instanceof Boolean bool)) {
      throw SDKError.validation("invocation_result", field + " must be a boolean");
    }
    return bool;
  }

  private static String string(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    if (!(value instanceof String string) || string.isBlank()) {
      throw SDKError.validation("invocation_result", field + " is required");
    }
    return string;
  }

  private static Map<String, Object> copyStringMap(Map<?, ?> map) {
    java.util.LinkedHashMap<String, Object> out = new java.util.LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : map.entrySet()) {
      if (entry.getKey() instanceof String key) {
        out.put(key, entry.getValue());
      }
    }
    return Map.copyOf(out);
  }

  private static String camelEnum(String state) {
    return switch (state) {
      case "Completed" -> "COMPLETED";
      case "Failed" -> "FAILED";
      case "Cancelled" -> "CANCELLED";
      case "TimedOut" -> "TIMED_OUT";
      case "BackpressureTerminated" -> "BACKPRESSURE_TERMINATED";
      default -> throw SDKError.validation("invocation_result", "unknown terminal state " + state);
    };
  }
}
