package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record CompatibilityFileRequest(
    CompatibilityCarrierBase base,
    String id,
    String fileRef,
    String ownerURA,
    String filename,
    String purpose,
    String contentType,
    String contentHash,
    long sizeBytes,
    long createdAt) {
  public CompatibilityFileRequest {
    id = CompatibilitySupport.requiredString(id, "id");
    fileRef = CompatibilitySupport.requiredURA(fileRef, "file_ref");
    ownerURA = CompatibilitySupport.requiredURA(ownerURA, "owner_ura");
    filename = CompatibilitySupport.requiredString(filename, "filename");
    purpose = CompatibilitySupport.requiredString(purpose, "purpose");
    contentType = CompatibilitySupport.requiredString(contentType, "content_type");
    contentHash = CompatibilitySupport.requiredString(contentHash, "content_hash");
    CompatibilitySupport.validateHash(contentHash, "content_hash");
    if (sizeBytes < 0 || createdAt < 0) {
      throw CompatibilitySupport.invalid("file facts must be non-negative");
    }
  }

  public static CompatibilityFileRequest fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "compatibility file request JSON");
    return new CompatibilityFileRequest(
        fields.containsKey("caller_ura") ? CompatibilityCarrierBase.fromObject(fields) : null,
        CompatibilitySupport.requiredString(fields.get("id"), "id"),
        CompatibilitySupport.requiredString(fields.get("file_ref"), "file_ref"),
        CompatibilitySupport.requiredString(fields.get("owner_ura"), "owner_ura"),
        CompatibilitySupport.requiredString(fields.get("filename"), "filename"),
        CompatibilitySupport.requiredString(fields.get("purpose"), "purpose"),
        CompatibilitySupport.requiredString(fields.get("content_type"), "content_type"),
        CompatibilitySupport.requiredString(fields.get("content_hash"), "content_hash"),
        CompatibilitySupport.requiredNonNegativeInteger(fields.get("size_bytes"), "size_bytes"),
        CompatibilitySupport.requiredNonNegativeInteger(fields.get("created_at"), "created_at"));
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = base == null ? new LinkedHashMap<>() : base.toObject();
    out.put("id", id);
    out.put("file_ref", fileRef);
    out.put("owner_ura", ownerURA);
    out.put("filename", filename);
    out.put("purpose", purpose);
    out.put("content_type", contentType);
    out.put("content_hash", contentHash);
    out.put("size_bytes", sizeBytes);
    out.put("created_at", createdAt);
    return JsonValueWriter.object(out);
  }
}
