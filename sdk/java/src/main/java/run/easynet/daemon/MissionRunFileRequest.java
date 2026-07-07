package run.easynet.daemon;

import java.util.LinkedHashMap;

public record MissionRunFileRequest(MissionCarrierBase carrier, String path, String label) {
  public MissionRunFileRequest {
    if (carrier == null) {
      throw MissionSupport.invalid("carrier is required");
    }
    path = MissionSupport.required(path, "path");
    label = MissionSupport.optional(label, "label");
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>(carrier.toObject());
    out.put("path", path);
    if (!label.isEmpty()) {
      out.put("label", label);
    }
    return JsonValueWriter.object(out);
  }
}
