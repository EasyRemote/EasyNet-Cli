package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record AdminGatewayStatusRequest(Boolean requirePublicListener, Map<String, Object> metadata) {
  public AdminGatewayStatusRequest {
    metadata = AdminSupport.copyObject(metadata);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    AdminSupport.putOptional(out, "require_public_listener", requirePublicListener);
    if (!metadata.isEmpty()) {
      out.put("metadata", metadata);
    }
    return JsonValueWriter.object(out);
  }
}
