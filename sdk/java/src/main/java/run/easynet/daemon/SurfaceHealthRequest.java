package run.easynet.daemon;

import java.util.LinkedHashMap;

public record SurfaceHealthRequest(SurfaceCarrierBase base, String projectID, String surfaceRef) {
  public SurfaceHealthRequest {
    if (base == null) {
      throw SurfaceSupport.invalid("surface carrier base is required");
    }
    projectID = SurfaceSupport.optionalClean(projectID, "project_id");
    if (projectID != null) {
      projectID = SurfaceSupport.projectID(projectID);
    }
    surfaceRef = SurfaceSupport.optionalClean(surfaceRef, "surface_ref");
    if (surfaceRef != null) {
      surfaceRef = SurfaceSupport.surfaceRef(surfaceRef);
    }
    if (projectID == null && surfaceRef == null) {
      throw SurfaceSupport.invalid("project_id or surface_ref is required");
    }
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = base.toObject();
    SurfaceSupport.putOptional(out, "project_id", projectID);
    SurfaceSupport.putOptional(out, "surface_ref", surfaceRef);
    return JsonValueWriter.object(out);
  }
}
