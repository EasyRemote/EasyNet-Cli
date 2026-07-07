package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record CompatibilityFileDeleteResult(
    String profile,
    String kind,
    String id,
    String object,
    boolean deleted,
    Map<String, Object> metadata) {
  public CompatibilityFileDeleteResult {
    CompatibilitySupport.validateKind(profile, kind, "file_delete_result");
    CompatibilitySupport.validateObject(object, "file", "file_delete_result");
    id = CompatibilitySupport.requiredString(id, "id");
    if (!deleted) {
      throw CompatibilitySupport.invalid("deleted must be true");
    }
    metadata = CompatibilitySupport.copyObject(metadata);
  }

  public static CompatibilityFileDeleteResult fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "compatibility file delete result JSON");
    Object deletedValue = fields.get("deleted");
    if (!(deletedValue instanceof Boolean deletedBool)) {
      throw CompatibilitySupport.invalid("deleted must be true");
    }
    return new CompatibilityFileDeleteResult(
        CompatibilitySupport.requiredString(fields.get("profile"), "profile"),
        CompatibilitySupport.requiredString(fields.get("kind"), "kind"),
        CompatibilitySupport.requiredString(fields.get("id"), "id"),
        CompatibilitySupport.requiredString(fields.get("object"), "object"),
        deletedBool,
        CompatibilitySupport.requiredObject(fields.get("metadata"), "metadata"));
  }

  public byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("profile", profile);
    out.put("kind", kind);
    out.put("id", id);
    out.put("object", object);
    out.put("deleted", deleted);
    out.put("metadata", metadata);
    return JsonValueWriter.object(out);
  }
}
