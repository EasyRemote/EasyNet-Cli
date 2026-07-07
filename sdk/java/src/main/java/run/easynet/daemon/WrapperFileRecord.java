package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record WrapperFileRecord(
    String profile,
    String kind,
    String fileRef,
    String ownerURA,
    String contentType,
    Long sizeBytes,
    String contentHash,
    Map<String, Object> metadata) {
  public WrapperFileRecord {
    WrapperSupport.validateKind(profile, kind, "file_record");
    fileRef = WrapperSupport.requiredURA(fileRef, "file_ref");
    ownerURA = WrapperSupport.requiredURA(ownerURA, "owner_ura");
    contentType = WrapperSupport.requiredString(contentType, "content_type");
    if (sizeBytes != null && sizeBytes < 0) {
      throw WrapperSupport.invalid("size_bytes must be non-negative");
    }
    contentHash = WrapperSupport.optionalString(contentHash, "content_hash");
    metadata = WrapperSupport.copyObject(metadata);
  }

  public static WrapperFileRecord fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "wrapper file record JSON");
    return new WrapperFileRecord(
        WrapperSupport.requiredString(fields.get("profile"), "profile"),
        WrapperSupport.requiredString(fields.get("kind"), "kind"),
        WrapperSupport.requiredString(fields.get("file_ref"), "file_ref"),
        WrapperSupport.requiredString(fields.get("owner_ura"), "owner_ura"),
        WrapperSupport.requiredString(fields.get("content_type"), "content_type"),
        WrapperSupport.optionalNonNegativeInteger(fields.get("size_bytes"), "size_bytes"),
        WrapperSupport.optionalString(fields.get("content_hash"), "content_hash"),
        WrapperSupport.requiredObject(fields.get("metadata"), "metadata"));
  }

  public byte[] toJSON() {
    LinkedHashMap<String, Object> object = new LinkedHashMap<>();
    object.put("profile", profile);
    object.put("kind", kind);
    object.put("file_ref", fileRef);
    object.put("owner_ura", ownerURA);
    object.put("content_type", contentType);
    object.put("size_bytes", sizeBytes);
    object.put("content_hash", contentHash);
    object.put("metadata", metadata);
    return JsonValueWriter.object(object);
  }
}
