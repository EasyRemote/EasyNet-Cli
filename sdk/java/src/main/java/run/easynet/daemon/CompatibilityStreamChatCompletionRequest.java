package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record CompatibilityStreamChatCompletionRequest(CompatibilityCarrierBase base, Map<String, Object> request) {
  public CompatibilityStreamChatCompletionRequest {
    if (base == null) {
      throw CompatibilitySupport.invalid("complete compatibility invocation carrier is required");
    }
    request = CompatibilityChatCompletionRequest.validateRequest(request, true);
  }

  public static CompatibilityStreamChatCompletionRequest fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "compatibility stream chat completion request JSON");
    return new CompatibilityStreamChatCompletionRequest(CompatibilityCarrierBase.fromObject(fields), CompatibilitySupport.requiredObject(fields.get("request"), "request"));
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = base.toObject();
    out.put("request", request);
    return JsonValueWriter.object(out);
  }
}
