package run.easynet.daemon;

import java.util.LinkedHashMap;

public record SurfaceManifestRequest(SurfaceCarrierBase base, String projectID) {
  public SurfaceManifestRequest {
    if (base == null) {
      throw SurfaceSupport.invalid("surface carrier base is required");
    }
    projectID = SurfaceSupport.projectID(projectID);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = base.toObject();
    out.put("project_id", projectID);
    return JsonValueWriter.object(out);
  }
}
