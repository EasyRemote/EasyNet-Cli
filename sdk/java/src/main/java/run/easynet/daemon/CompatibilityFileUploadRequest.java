package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record CompatibilityFileUploadRequest(
    CompatibilityCarrierBase base,
    String id,
    String fileRef,
    String ownerURA,
    String filename,
    String purpose,
    String contentType,
    String contentHash,
    long sizeBytes,
    long createdAt,
    Map<String, Object> metadata) {
  public CompatibilityFileUploadRequest {
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
    metadata = CompatibilitySupport.copyObject(metadata);
  }

  public static CompatibilityFileUploadRequest fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "compatibility file upload request JSON");
    return new CompatibilityFileUploadRequest(
        fields.containsKey("caller_ura") ? CompatibilityCarrierBase.fromObject(fields) : null,
        CompatibilitySupport.requiredString(fields.get("id"), "id"),
        CompatibilitySupport.requiredString(fields.get("file_ref"), "file_ref"),
        CompatibilitySupport.requiredString(fields.get("owner_ura"), "owner_ura"),
        CompatibilitySupport.requiredString(fields.get("filename"), "filename"),
        CompatibilitySupport.requiredString(fields.get("purpose"), "purpose"),
        CompatibilitySupport.requiredString(fields.get("content_type"), "content_type"),
        CompatibilitySupport.requiredString(fields.get("content_hash"), "content_hash"),
        CompatibilitySupport.requiredNonNegativeInteger(fields.get("size_bytes"), "size_bytes"),
        CompatibilitySupport.requiredNonNegativeInteger(fields.get("created_at"), "created_at"),
        fields.containsKey("metadata") ? CompatibilitySupport.requiredObject(fields.get("metadata"), "metadata") : Map.of());
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
    mergeMetadata(out, metadata);
    return JsonValueWriter.object(out);
  }

  static void mergeMetadata(LinkedHashMap<String, Object> out, Map<String, Object> extra) {
    if (extra == null || extra.isEmpty()) {
      return;
    }
    LinkedHashMap<String, Object> merged = new LinkedHashMap<>();
    Object existing = out.get("metadata");
    if (existing instanceof Map<?, ?> map) {
      for (Map.Entry<?, ?> entry : map.entrySet()) {
        if (entry.getKey() instanceof String key) {
          merged.put(key, entry.getValue());
        }
      }
    }
    merged.putAll(extra);
    out.put("metadata", merged);
  }
}
