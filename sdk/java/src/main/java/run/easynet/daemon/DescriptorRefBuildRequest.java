package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record DescriptorRefBuildRequest(
    String abilityURA, String descriptorVersion, Map<String, Object> metadata) {
  public DescriptorRefBuildRequest {
    abilityURA = DirectoryIdentitySupport.cleanRequired(abilityURA, "ability_ura");
    descriptorVersion =
        DirectoryIdentitySupport.cleanRequired(descriptorVersion, "descriptor_version");
    metadata = DirectoryIdentitySupport.copyObject(metadata);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("ability_ura", abilityURA);
    out.put("descriptor_version", descriptorVersion);
    if (!metadata.isEmpty()) {
      out.put("metadata", metadata);
    }
    return JsonValueWriter.object(out);
  }
}
