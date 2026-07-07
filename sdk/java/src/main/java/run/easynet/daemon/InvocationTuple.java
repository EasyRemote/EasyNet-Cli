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
    String argsJson,
    Map<String, Object> metadata) {
  public InvocationTuple(
      String caller,
      String callee,
      String descriptor,
      String subject,
      String nonce,
      String causalContext,
      String argsJson) {
    this(caller, callee, descriptor, subject, nonce, causalContext, argsJson, Map.of());
  }

  public InvocationTuple {
    caller = required(caller, "caller");
    callee = required(callee, "callee");
    descriptor = required(descriptor, "descriptor");
    subject = required(subject, "subject");
    nonce = required(nonce, "nonce");
    causalContext = required(causalContext, "causalContext");
    argsJson = required(argsJson, "argsJson");
    metadata = copyObject(metadata);
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
    out.put("metadata", metadata);
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
        JsonValueWriter.write(requiredValue(fields, "args")),
        optionalObject(fields.get("metadata"), "metadata"));
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

  private static Map<String, Object> optionalObject(Object value, String field) {
    if (value == null) {
      return Map.of();
    }
    if (!(value instanceof Map<?, ?> raw)) {
      throw SDKError.validation("invocation", field + " must be an object");
    }
    Map<String, Object> out = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : raw.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw SDKError.validation("invocation", field + " keys must be strings");
      }
      out.put(key, entry.getValue());
    }
    return Map.copyOf(out);
  }

  private static Map<String, Object> copyObject(Map<String, Object> value) {
    if (value == null || value.isEmpty()) {
      return Map.of();
    }
    return Map.copyOf(new LinkedHashMap<>(value));
  }

  private static String required(String value, String field) {
    if (value == null || value.isBlank()) {
      throw SDKError.validation("invocation", field + " is required");
    }
    return value;
  }
}
