package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record MissionRunRequest(MissionCarrierBase carrier, String source, String label) {
  public MissionRunRequest {
    if (carrier == null) {
      throw MissionSupport.invalid("carrier is required");
    }
    source = MissionSupport.required(source, "source");
    label = MissionSupport.optional(label, "label");
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>(carrier.toObject());
    out.put("source", source);
    if (!label.isEmpty()) {
      out.put("label", label);
    }
    return JsonValueWriter.object(out);
  }
}
