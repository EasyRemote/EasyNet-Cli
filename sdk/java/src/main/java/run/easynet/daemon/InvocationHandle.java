package run.easynet.daemon;

import java.util.Map;

public final class InvocationHandle {
  private final long handleId;
  private final String state;
  private final boolean terminal;
  private RuntimeClient runtime;

  public InvocationHandle(long handleId, String state, boolean terminal) {
    if (handleId <= 0) {
      throw SDKError.validation("invocation_handle", "handle_id must be positive");
    }
    if (state == null || state.isBlank()) {
      throw SDKError.validation("invocation_handle", "state is required");
    }
    this.handleId = handleId;
    this.state = state;
    this.terminal = terminal;
  }

  public static InvocationHandle fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "invocation handle");
    rejectUnknown(fields, "handle_id", "state", "terminal", "events", "result");
    return new InvocationHandle(positiveLong(fields, "handle_id"), string(fields, "state"), bool(fields, "terminal"));
  }

  InvocationHandle bindRuntime(RuntimeClient runtime) {
    this.runtime = runtime;
    return this;
  }

  public long handleId() {
    return handleId;
  }

  public String state() {
    return state;
  }

  public boolean terminal() {
    return terminal;
  }

  RuntimeClient runtime() {
    return runtime;
  }

  private static void rejectUnknown(Map<String, Object> fields, String... allowed) {
    java.util.Set<String> allowedSet = java.util.Set.of(allowed);
    for (String key : fields.keySet()) {
      if (!allowedSet.contains(key)) {
        throw SDKError.validation("invocation_handle", key + " is not supported");
      }
    }
  }

  private static long positiveLong(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    long number;
    if (value instanceof Long longValue) {
      number = longValue;
    } else if (value instanceof Integer integerValue) {
      number = integerValue.longValue();
    } else {
      throw SDKError.validation("invocation_handle", field + " must be an integer");
    }
    if (number <= 0) {
      throw SDKError.validation("invocation_handle", field + " must be positive");
    }
    return number;
  }

  private static String string(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    if (!(value instanceof String string) || string.isBlank()) {
      throw SDKError.validation("invocation_handle", field + " is required");
    }
    return string;
  }

  private static boolean bool(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    if (!(value instanceof Boolean bool)) {
      throw SDKError.validation("invocation_handle", field + " must be a boolean");
    }
    return bool;
  }
}
