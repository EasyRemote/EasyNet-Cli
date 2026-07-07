package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record CompatibilityFile(
    String profile,
    String kind,
    String id,
    String object,
    long bytes,
    long createdAt,
    String filename,
    String purpose,
    String status,
    Map<String, Object> metadata) {
  public CompatibilityFile {
    CompatibilitySupport.validateKind(profile, kind, "file");
    CompatibilitySupport.validateObject(object, "file", "file");
    id = CompatibilitySupport.requiredString(id, "id");
    if (bytes < 0 || createdAt < 0) {
      throw CompatibilitySupport.invalid("file byte and created_at values must be non-negative");
    }
    filename = CompatibilitySupport.requiredString(filename, "filename");
    purpose = CompatibilitySupport.requiredString(purpose, "purpose");
    status = CompatibilitySupport.requiredString(status, "status");
    metadata = CompatibilitySupport.copyObject(metadata);
  }

  public static CompatibilityFile fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "compatibility file JSON");
    return new CompatibilityFile(
        CompatibilitySupport.requiredString(fields.get("profile"), "profile"),
        CompatibilitySupport.requiredString(fields.get("kind"), "kind"),
        CompatibilitySupport.requiredString(fields.get("id"), "id"),
        CompatibilitySupport.requiredString(fields.get("object"), "object"),
        CompatibilitySupport.requiredNonNegativeInteger(fields.get("bytes"), "bytes"),
        CompatibilitySupport.requiredNonNegativeInteger(fields.get("created_at"), "created_at"),
        CompatibilitySupport.requiredString(fields.get("filename"), "filename"),
        CompatibilitySupport.requiredString(fields.get("purpose"), "purpose"),
        CompatibilitySupport.requiredString(fields.get("status"), "status"),
        CompatibilitySupport.requiredObject(fields.get("metadata"), "metadata"));
  }

  public byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("profile", profile);
    out.put("kind", kind);
    out.put("id", id);
    out.put("object", object);
    out.put("bytes", bytes);
    out.put("created_at", createdAt);
    out.put("filename", filename);
    out.put("purpose", purpose);
    out.put("status", status);
    out.put("metadata", metadata);
    return JsonValueWriter.object(out);
  }
}
