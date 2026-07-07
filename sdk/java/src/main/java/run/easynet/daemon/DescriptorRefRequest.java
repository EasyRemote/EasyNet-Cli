package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record DescriptorRefRequest(String descriptorRef, Map<String, Object> metadata) {
  public DescriptorRefRequest {
    descriptorRef = DirectoryIdentitySupport.cleanRequired(descriptorRef, "descriptor_ref");
    metadata = DirectoryIdentitySupport.copyObject(metadata);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("descriptor_ref", descriptorRef);
    if (!metadata.isEmpty()) {
      out.put("metadata", metadata);
    }
    return JsonValueWriter.object(out);
  }
}
