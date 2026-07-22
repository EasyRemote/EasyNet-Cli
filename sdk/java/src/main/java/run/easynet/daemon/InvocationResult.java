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
    validateTerminalReceipt(terminalReceipt, terminalState, ok);
    if (ok && error != null) {
      throw SDKError.validation("runtime", "ok result must not carry error");
    }
    if (!ok && error == null) {
      throw SDKError.validation("runtime", "failed result must carry error");
    }
  }

  static InvocationResult fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "invocation result");
    if (fields.containsKey("receipt")) {
      throw SDKError.validation(
          "invocation_result",
          "invocation result must use terminal_receipt; retired receipt alias is not accepted");
    }
    boolean ok = bool(fields, "ok");
    String state = string(fields, "terminal_state");
    Object output = fields.get("output_json");
    Map<String, Object> terminalReceipt = optionalReceipt(fields, "terminal_receipt");
    SDKError error = ok ? null : SDKError.validation("runtime", "invocation failed");
    return new InvocationResult(
        ok,
        InvocationTerminalState.valueOf(camelEnum(state)),
        output == null ? "" : JsonValueWriter.write(output),
        error,
        terminalReceipt);
  }

  private static Map<String, Object> optionalReceipt(Map<String, Object> fields, String field) {
    if (!fields.containsKey(field) || fields.get(field) == null) {
      return Map.of();
    }
    Object value = fields.get(field);
    if (!(value instanceof Map<?, ?> map)) {
      throw SDKError.validation("invocation_result", field + " must be an object");
    }
    return copyStringMap(map);
  }

  private static void validateTerminalReceipt(
      Map<String, Object> terminalReceipt, InvocationTerminalState terminalState, boolean ok) {
    if (terminalReceipt.isEmpty()) {
      return;
    }
    RuntimeReceipt receipt = RuntimeReceipt.fromMap(terminalReceipt);
    String receiptState = receipt.lifecycleState();
    String resultState =
        switch (terminalState) {
          case COMPLETED -> "COMPLETED";
          case FAILED -> "FAILED";
          case CANCELLED -> "CANCELLED";
          case TIMED_OUT -> "TIMED_OUT";
          case BACKPRESSURE_TERMINATED -> "BACKPRESSURE_TERMINATED";
        };
    if (!receiptState.equals(resultState)) {
      throw SDKError.validation(
          "terminal_receipt", "terminal_receipt state does not match invocation terminal_state");
    }
    if (ok != receiptState.equals("COMPLETED")) {
      throw SDKError.validation(
          "terminal_receipt", "invocation result ok flag does not match terminal receipt state");
    }
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
      } else {
        throw SDKError.validation("invocation_result", "receipt object keys must be strings");
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
