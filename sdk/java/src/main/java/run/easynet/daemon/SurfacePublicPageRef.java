package run.easynet.daemon;

import java.util.Map;

public record SurfacePublicPageRef(
    String profile,
    String kind,
    String pageID,
    String ownerURA,
    String surfaceRef,
    String publicRef,
    String routeKind,
    Map<String, Object> metadata) {
  public SurfacePublicPageRef {
    if (!SurfaceSupport.PROFILE.equals(profile)
        || !"public_page_ref".equals(kind)
        || !"hub_web".equals(routeKind)) {
      throw SurfaceSupport.invalid("invalid surface public page ref projection");
    }
    pageID = SurfaceSupport.cleanRequired(pageID, "page_id");
    ownerURA = SurfaceSupport.cleanRequired(ownerURA, "owner_ura");
    surfaceRef = SurfaceSupport.surfaceRef(surfaceRef);
    publicRef = SurfaceSupport.cleanRequired(publicRef, "public_ref");
    metadata = SurfaceSupport.copyObject(metadata);
  }

  public static SurfacePublicPageRef fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "surface public page ref JSON");
    return new SurfacePublicPageRef(
        SurfaceSupport.requiredString(fields, "profile"),
        SurfaceSupport.requiredString(fields, "kind"),
        SurfaceSupport.requiredString(fields, "page_id"),
        SurfaceSupport.requiredString(fields, "owner_ura"),
        SurfaceSupport.requiredString(fields, "surface_ref"),
        SurfaceSupport.requiredString(fields, "public_ref"),
        SurfaceSupport.requiredString(fields, "route_kind"),
        SurfaceSupport.requiredObject(fields, "metadata"));
  }
}
