package run.runtime.sdk;

import java.util.Map;
import java.util.Set;

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
    if (terminalReceipt == null || terminalReceipt.isEmpty()) {
      throw SDKError.validation("invocation_result", "terminal_receipt is required");
    }
    RuntimeReceipt receipt = validatedRuntimeReceipt(terminalReceipt, terminalState, ok);
    terminalReceipt = receipt.rawProjection();
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
    requireExactKeys(fields, "ok", "terminal_state", "output_json", "terminal_receipt");
    boolean ok = bool(fields, "ok");
    String state = string(fields, "terminal_state");
    Object output = fields.get("output_json");
    Map<String, Object> terminalReceipt = requiredTerminalReceipt(fields);
    SDKError error = ok ? null : SDKError.validation("runtime", "invocation failed");
    return new InvocationResult(
        ok,
        InvocationTerminalState.valueOf(camelEnum(state)),
        output == null ? "" : JsonValueWriter.write(output),
        error,
        terminalReceipt);
  }

  public RuntimeReceipt runtimeReceipt() {
    return RuntimeReceipt.fromMap(terminalReceipt);
  }

  private static Map<String, Object> requiredTerminalReceipt(Map<String, Object> fields) {
    if (!fields.containsKey("terminal_receipt") || fields.get("terminal_receipt") == null) {
      throw SDKError.validation("invocation_result", "terminal_receipt is required");
    }
    Object value = fields.get("terminal_receipt");
    if (!(value instanceof Map<?, ?> map)) {
      throw SDKError.validation("invocation_result", "terminal_receipt must be an object");
    }
    return copyStringMap(map);
  }

  private static RuntimeReceipt validatedRuntimeReceipt(
      Map<String, Object> terminalReceipt, InvocationTerminalState terminalState, boolean ok) {
    RuntimeReceipt receipt = RuntimeReceipt.fromMap(terminalReceipt);
    String receiptState = receipt.lifecycleState();
    String resultState =
        switch (terminalState) {
          case COMPLETED -> "COMPLETED";
          case FAILED -> "FAILED";
          case CANCELLED -> "CANCELLED";
          case TIMED_OUT -> "TIMED_OUT";
        };
    if (!receiptState.equals(resultState)) {
      throw SDKError.validation(
          "terminal_receipt", "terminal_receipt state does not match invocation terminal_state");
    }
    if (ok != receiptState.equals("COMPLETED")) {
      throw SDKError.validation(
          "terminal_receipt", "invocation result ok flag does not match terminal receipt state");
    }
    return receipt;
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

  private static void requireExactKeys(Map<String, Object> fields, String... allowedKeys) {
    Set<String> allowed = Set.of(allowedKeys);
    for (String key : fields.keySet()) {
      if (!allowed.contains(key)) {
        throw SDKError.validation(
            "invocation_result",
            "invocation result contains noncanonical field " + key);
      }
    }
  }

  private static String camelEnum(String state) {
    return switch (state) {
      case "Completed" -> "COMPLETED";
      case "Failed" -> "FAILED";
      case "Cancelled" -> "CANCELLED";
      case "TimedOut" -> "TIMED_OUT";
      default -> throw SDKError.validation("invocation_result", "unknown terminal state " + state);
    };
  }
}
