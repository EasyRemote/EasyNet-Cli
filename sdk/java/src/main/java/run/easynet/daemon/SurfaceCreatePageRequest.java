package run.easynet.daemon;

import java.util.LinkedHashMap;

public record SurfaceCreatePageRequest(
    SurfaceCarrierBase base, String projectID, String folder, String visibility) {
  public SurfaceCreatePageRequest {
    if (base == null) {
      throw SurfaceSupport.invalid("surface carrier base is required");
    }
    projectID = SurfaceSupport.projectID(projectID);
    folder = SurfaceSupport.cleanRequired(folder, "folder");
    if (!folder.startsWith("/")) {
      throw SurfaceSupport.invalid("surface folder must be absolute");
    }
    visibility = visibility == null || visibility.isEmpty() ? "public" : visibility;
    if (!visibility.equals("public") && !visibility.equals("private")) {
      throw SurfaceSupport.invalid("invalid surface visibility");
    }
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = base.toObject();
    out.put("project_id", projectID);
    out.put("folder", folder);
    out.put("visibility", visibility);
    return JsonValueWriter.object(out);
  }
}
