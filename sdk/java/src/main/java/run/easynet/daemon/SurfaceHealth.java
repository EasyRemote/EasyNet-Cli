package run.easynet.daemon;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

public record SurfaceHealth(
    String profile,
    String kind,
    String state,
    boolean ready,
    String ownerURA,
    String surfaceRef,
    String descriptorRef,
    String descriptorVersion,
    int pageCount,
    List<SurfaceHealthCheck> checks,
    Map<String, Object> metadata) {
  public SurfaceHealth {
    if (!SurfaceSupport.PROFILE.equals(profile) || !"surface_health".equals(kind)) {
      throw SurfaceSupport.invalid("invalid surface health projection");
    }
    state = SurfaceSupport.cleanRequired(state, "state");
    ownerURA = SurfaceSupport.cleanRequired(ownerURA, "owner_ura");
    surfaceRef = SurfaceSupport.surfaceRef(surfaceRef);
    descriptorRef = SurfaceSupport.cleanRequired(descriptorRef, "descriptor_ref");
    descriptorVersion = SurfaceSupport.cleanRequired(descriptorVersion, "descriptor_version");
    if (pageCount < 0) {
      throw SurfaceSupport.invalid("page_count must be non-negative");
    }
    checks = checks == null ? List.of() : List.copyOf(checks);
    metadata = SurfaceSupport.copyObject(metadata);
  }

  public static SurfaceHealth fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "surface health JSON");
    List<SurfaceHealthCheck> checks = new ArrayList<>();
    for (Object item : SurfaceSupport.requiredList(fields, "checks")) {
      Map<String, Object> checkObject = SurfaceSupport.optionalObject(item, "checks");
      if (checkObject == null) {
        throw SurfaceSupport.invalid("checks entry must be an object");
      }
      checks.add(SurfaceHealthCheck.fromObject(checkObject));
    }
    return new SurfaceHealth(
        SurfaceSupport.requiredString(fields, "profile"),
        SurfaceSupport.requiredString(fields, "kind"),
        SurfaceSupport.requiredString(fields, "state"),
        SurfaceSupport.requiredBoolean(fields, "ready"),
        SurfaceSupport.requiredString(fields, "owner_ura"),
        SurfaceSupport.requiredString(fields, "surface_ref"),
        SurfaceSupport.requiredString(fields, "descriptor_ref"),
        SurfaceSupport.requiredString(fields, "descriptor_version"),
        SurfaceSupport.requiredInteger(fields, "page_count"),
        checks,
        SurfaceSupport.requiredObject(fields, "metadata"));
  }
}
