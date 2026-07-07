package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record CompatibilityChatCompletionRequest(CompatibilityCarrierBase base, Map<String, Object> request) {
  public CompatibilityChatCompletionRequest {
    if (base == null) {
      throw CompatibilitySupport.invalid("complete compatibility invocation carrier is required");
    }
    request = validateRequest(request, false);
  }

  public static CompatibilityChatCompletionRequest fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "compatibility chat completion request JSON");
    return new CompatibilityChatCompletionRequest(CompatibilityCarrierBase.fromObject(fields), CompatibilitySupport.requiredObject(fields.get("request"), "request"));
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = base.toObject();
    out.put("request", request);
    return JsonValueWriter.object(out);
  }

  static Map<String, Object> validateRequest(Map<String, Object> raw, boolean stream) {
    LinkedHashMap<String, Object> copy = new LinkedHashMap<>(CompatibilitySupport.copyObject(raw));
    copy.put("model", CompatibilitySupport.requiredAbilityURA(copy.get("model"), "model"));
    if (CompatibilitySupport.requiredList(copy.get("messages"), "messages").isEmpty()) {
      throw CompatibilitySupport.invalid("messages must be a non-empty array");
    }
    if (!stream && Boolean.TRUE.equals(copy.get("stream"))) {
      throw CompatibilitySupport.invalid("unary chat completion request must not set stream=true");
    }
    if (stream) {
      copy.put("stream", true);
    }
    return Map.copyOf(copy);
  }
}
