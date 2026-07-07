package run.easynet.daemon;

import java.nio.charset.StandardCharsets;
import java.util.LinkedHashMap;
import java.util.Map;

public record InvocationTuple(
    String caller,
    String callee,
    String descriptor,
    String subject,
    String nonce,
    String causalContext,
    String argsJson) {
  public InvocationTuple {
    caller = required(caller, "caller");
    callee = required(callee, "callee");
    descriptor = required(descriptor, "descriptor");
    subject = required(subject, "subject");
    nonce = required(nonce, "nonce");
    causalContext = required(causalContext, "causalContext");
    argsJson = required(argsJson, "argsJson");
  }

  Map<String, Object> toWireObject() {
    Map<String, Object> out = new LinkedHashMap<>();
    out.put("caller_ura", caller);
    out.put("callee_ura", callee);
    out.put("descriptor_ref", descriptor);
    out.put("subject_ura", subject);
    out.put("nonce_base64", nonce);
    out.put("causal_context", decodeJSONValue(causalContext, "causal_context"));
    out.put("args", decodeJSONValue(argsJson, "args"));
    out.put("content_type", "application/json");
    out.put("metadata", Map.of());
    return out;
  }

  static InvocationTuple fromWireObject(Map<String, Object> fields) {
    return new InvocationTuple(
        string(fields, "caller_ura"),
        string(fields, "callee_ura"),
        string(fields, "descriptor_ref"),
        string(fields, "subject_ura"),
        string(fields, "nonce_base64"),
        JsonValueWriter.write(requiredValue(fields, "causal_context")),
        JsonValueWriter.write(requiredValue(fields, "args")));
  }

  private static Object decodeJSONValue(String raw, String field) {
    return JsonValueReader.value(raw.getBytes(StandardCharsets.UTF_8), field);
  }

  private static String string(Map<String, Object> fields, String field) {
    Object value = requiredValue(fields, field);
    if (!(value instanceof String string) || string.isBlank()) {
      throw SDKError.validation("invocation", field + " is required");
    }
    return string;
  }

  private static Object requiredValue(Map<String, Object> fields, String field) {
    if (!fields.containsKey(field) || fields.get(field) == null) {
      throw SDKError.validation("invocation", field + " is required");
    }
    return fields.get(field);
  }

  private static String required(String value, String field) {
    if (value == null || value.isBlank()) {
      throw SDKError.validation("invocation", field + " is required");
    }
    return value;
  }
}
