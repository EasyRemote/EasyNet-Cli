package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record CompatibilityFileDeleteRequest(CompatibilityCarrierBase base, String id, boolean deleted) {
  public CompatibilityFileDeleteRequest {
    id = CompatibilitySupport.requiredString(id, "id");
    if (!deleted) {
      throw CompatibilitySupport.invalid("deleted must be true");
    }
  }

  public static CompatibilityFileDeleteRequest fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "compatibility file delete request JSON");
    return new CompatibilityFileDeleteRequest(
        fields.containsKey("caller_ura") ? CompatibilityCarrierBase.fromObject(fields) : null,
        CompatibilitySupport.requiredString(fields.get("id"), "id"),
        CompatibilitySupport.requiredTrue(fields.get("deleted"), "deleted"));
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = base == null ? new LinkedHashMap<>() : base.toObject();
    out.put("id", id);
    out.put("deleted", deleted);
    return JsonValueWriter.object(out);
  }
}
