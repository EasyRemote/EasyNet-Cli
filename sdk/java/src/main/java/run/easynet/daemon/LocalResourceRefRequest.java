package run.easynet.daemon;

import java.util.LinkedHashMap;

public record LocalResourceRefRequest(String path, String capability) {
  public LocalResourceRefRequest {
    path = PublicationSupport.absolutePath(path);
    capability = PublicationSupport.capability(capability);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("path", path);
    out.put("capability", capability);
    return JsonValueWriter.object(out);
  }
}
