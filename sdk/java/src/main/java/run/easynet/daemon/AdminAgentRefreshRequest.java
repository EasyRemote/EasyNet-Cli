package run.easynet.daemon;

import java.util.LinkedHashMap;

public record AdminAgentRefreshRequest(AdminCarrierBase carrier, String name) {
  public AdminAgentRefreshRequest {
    if (carrier == null) {
      throw AdminSupport.invalid("carrier is required");
    }
    name = AdminSupport.optionalAgentName(name);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>(carrier.toObject());
    AdminSupport.putOptional(out, "name", name);
    return JsonValueWriter.object(out);
  }
}
