package run.easynet.daemon;

import java.util.Map;

public record SurfaceMutationResult(
    String profile,
    String kind,
    String operation,
    String pageID,
    boolean removed,
    String state,
    Map<String, Object> metadata) {
  public SurfaceMutationResult {
    if (!SurfaceSupport.PROFILE.equals(profile)
        || !"surface_mutation_result".equals(kind)
        || !"delete".equals(operation)
        || (!"deleted".equals(state) && !"unknown".equals(state))) {
      throw SurfaceSupport.invalid("invalid surface mutation result projection");
    }
    pageID = SurfaceSupport.projectID(pageID);
    metadata = SurfaceSupport.copyObject(metadata);
  }

  public static SurfaceMutationResult fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "surface mutation result JSON");
    return new SurfaceMutationResult(
        SurfaceSupport.requiredString(fields, "profile"),
        SurfaceSupport.requiredString(fields, "kind"),
        SurfaceSupport.requiredString(fields, "operation"),
        SurfaceSupport.requiredString(fields, "page_id"),
        SurfaceSupport.requiredBoolean(fields, "removed"),
        SurfaceSupport.requiredString(fields, "state"),
        SurfaceSupport.requiredObject(fields, "metadata"));
  }
}
