package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record SurfacePageRecord(
    String profile,
    String kind,
    String pageID,
    String ownerURA,
    String surfaceRef,
    String publicRef,
    String status,
    Map<String, Object> metadata) {
  public SurfacePageRecord {
    if (!SurfaceSupport.PROFILE.equals(profile) || !"page_record".equals(kind)) {
      throw SurfaceSupport.invalid("invalid surface page record projection");
    }
    pageID = SurfaceSupport.cleanRequired(pageID, "page_id");
    ownerURA = SurfaceSupport.cleanRequired(ownerURA, "owner_ura");
    surfaceRef = SurfaceSupport.surfaceRef(surfaceRef);
    publicRef = SurfaceSupport.optionalClean(publicRef, "public_ref");
    status = SurfaceSupport.optionalClean(status, "status");
    metadata = SurfaceSupport.copyObject(metadata);
  }

  static SurfacePageRecord fromObject(Map<String, Object> fields) {
    return new SurfacePageRecord(
        SurfaceSupport.requiredString(fields, "profile"),
        SurfaceSupport.requiredString(fields, "kind"),
        SurfaceSupport.requiredString(fields, "page_id"),
        SurfaceSupport.requiredString(fields, "owner_ura"),
        SurfaceSupport.requiredString(fields, "surface_ref"),
        SurfaceSupport.optionalString(fields.get("public_ref"), "public_ref"),
        SurfaceSupport.optionalString(fields.get("status"), "status"),
        SurfaceSupport.requiredObject(fields, "metadata"));
  }

  public static SurfacePageRecord fromJSON(byte[] raw) {
    return fromObject(JsonValueReader.object(raw, "surface page record JSON"));
  }

  Map<String, Object> toObject() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("profile", profile);
    out.put("kind", kind);
    out.put("page_id", pageID);
    out.put("owner_ura", ownerURA);
    out.put("surface_ref", surfaceRef);
    out.put("public_ref", publicRef);
    out.put("status", status);
    out.put("metadata", metadata);
    return out;
  }
}
