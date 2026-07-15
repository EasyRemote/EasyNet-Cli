package run.easynet.daemon;

import java.util.Map;

public final class InvocationHandle {
  private final InvocationControlCapability control;
  private final String state;
  private final boolean terminal;
  private RuntimeClient runtime;

  InvocationHandle(long handleId, String state, boolean terminal) {
    this(InvocationControlCapability.runtimeBound(handleId), state, terminal);
  }

  InvocationHandle(InvocationControlCapability control, String state, boolean terminal) {
    if (control == null) {
      throw SDKError.validation("invocation_handle", "control capability is required");
    }
    if (state == null || state.isBlank()) {
      throw SDKError.validation("invocation_handle", "state is required");
    }
    this.control = control;
    this.state = state;
    this.terminal = terminal;
  }

  public static InvocationHandle fromJSON(byte[] raw) {
    return fromJSON(raw, null, false);
  }

  static InvocationHandle fromRuntimeJSON(byte[] raw) {
    return fromJSON(raw, null, true);
  }

  static InvocationHandle fromJSONWithControl(byte[] raw, InvocationControlCapability control) {
    return fromJSON(raw, control, false);
  }

  private static InvocationHandle fromJSON(
      byte[] raw, InvocationControlCapability expectedControl, boolean runtimeBound) {
    Map<String, Object> fields = JsonValueReader.object(raw, "invocation handle");
    rejectUnknown(fields, "handle_id", "state", "terminal", "events", "result");
    long handleId = positiveLong(fields, "handle_id");
    InvocationControlCapability control;
    if (expectedControl != null) {
      if (expectedControl.rawHandleId() != handleId) {
        throw SDKError.validation(
            "invocation_handle", "handle_id does not match invocation control capability");
      }
      control = expectedControl;
    } else if (runtimeBound) {
      control = InvocationControlCapability.runtimeBound(handleId);
    } else {
      control = InvocationControlCapability.snapshot(handleId);
    }
    return new InvocationHandle(
        control, string(fields, "state"), bool(fields, "terminal"));
  }

  InvocationHandle bindRuntime(RuntimeClient runtime) {
    this.runtime = runtime;
    return this;
  }

  public InvocationControlCapability controlCapability() {
    control.adapterHandleId();
    return control;
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

  static long positiveLong(Map<String, Object> fields, String field) {
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

  static String string(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    if (!(value instanceof String string) || string.isBlank()) {
      throw SDKError.validation("invocation_handle", field + " is required");
    }
    return string;
  }

  static boolean bool(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    if (!(value instanceof Boolean bool)) {
      throw SDKError.validation("invocation_handle", field + " must be a boolean");
    }
    return bool;
  }
}
