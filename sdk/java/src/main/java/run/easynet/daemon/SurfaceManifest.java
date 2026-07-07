package run.easynet.daemon;

import java.util.Map;

public record SurfaceManifest(
    String profile,
    String kind,
    String pageID,
    String ownerURA,
    String surfaceRef,
    String publicRef,
    SurfacePageRecord page,
    Map<String, Object> entrypoint,
    Map<String, Object> metadata) {
  public SurfaceManifest {
    if (!SurfaceSupport.PROFILE.equals(profile) || !"surface_manifest".equals(kind)) {
      throw SurfaceSupport.invalid("invalid surface manifest projection");
    }
    pageID = SurfaceSupport.cleanRequired(pageID, "page_id");
    ownerURA = SurfaceSupport.cleanRequired(ownerURA, "owner_ura");
    surfaceRef = SurfaceSupport.surfaceRef(surfaceRef);
    publicRef = SurfaceSupport.cleanRequired(publicRef, "public_ref");
    if (page == null || !page.pageID().equals(pageID) || !page.surfaceRef().equals(surfaceRef)) {
      throw SurfaceSupport.invalid("invalid surface manifest page projection");
    }
    entrypoint = SurfaceSupport.copyObject(entrypoint);
    if (entrypoint.isEmpty()) {
      throw SurfaceSupport.invalid("entrypoint must be an object");
    }
    metadata = SurfaceSupport.copyObject(metadata);
  }

  public static SurfaceManifest fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "surface manifest JSON");
    return new SurfaceManifest(
        SurfaceSupport.requiredString(fields, "profile"),
        SurfaceSupport.requiredString(fields, "kind"),
        SurfaceSupport.requiredString(fields, "page_id"),
        SurfaceSupport.requiredString(fields, "owner_ura"),
        SurfaceSupport.requiredString(fields, "surface_ref"),
        SurfaceSupport.requiredString(fields, "public_ref"),
        SurfacePageRecord.fromObject(SurfaceSupport.requiredObject(fields, "page")),
        SurfaceSupport.requiredObject(fields, "entrypoint"),
        SurfaceSupport.requiredObject(fields, "metadata"));
  }
}
