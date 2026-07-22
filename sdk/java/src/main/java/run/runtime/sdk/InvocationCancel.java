package run.runtime.sdk;

import java.util.Map;

public record InvocationCancel(
    InvocationControlCapability control,
    boolean requestAccepted,
    boolean deduplicated,
    boolean cancelled,
    String state,
    boolean terminal) {
  public InvocationCancel {
    if (control == null) {
      throw SDKError.validation("invocation_cancel", "control capability is required");
    }
    if (state == null || state.isBlank()) {
      throw SDKError.validation("invocation_cancel", "state is required");
    }
  }

  static InvocationCancel fromJSON(byte[] raw) {
    return fromJSON(raw, null);
  }

  static InvocationCancel fromJSONWithControl(byte[] raw, InvocationControlCapability expectedControl) {
    return fromJSON(raw, expectedControl);
  }

  private static InvocationCancel fromJSON(
      byte[] raw, InvocationControlCapability expectedControl) {
    Map<String, Object> fields = JsonValueReader.object(raw, "invocation cancel");
    long handleId = InvocationHandle.positiveLong(fields, "handle_id");
    InvocationControlCapability control;
    if (expectedControl != null) {
      if (expectedControl.rawHandleId() != handleId) {
        throw SDKError.validation(
            "invocation_cancel", "handle_id does not match invocation control capability");
      }
      control = expectedControl;
    } else {
      control = InvocationControlCapability.snapshot(handleId);
    }
    return new InvocationCancel(
        control,
        bool(fields, "request_accepted"),
        bool(fields, "deduplicated"),
        bool(fields, "cancelled"),
        InvocationHandle.string(fields, "state"),
        bool(fields, "terminal"));
  }

  private static boolean bool(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    if (!(value instanceof Boolean bool)) {
      throw SDKError.validation("invocation_cancel", field + " must be a boolean");
    }
    return bool;
  }
}
