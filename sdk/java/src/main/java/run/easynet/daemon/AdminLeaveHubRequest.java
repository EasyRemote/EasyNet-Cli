package run.easynet.daemon;

import java.util.LinkedHashMap;

public record AdminLeaveHubRequest(AdminCarrierBase carrier, String hubURA, String reason) {
  public AdminLeaveHubRequest {
    if (carrier == null) {
      throw AdminSupport.invalid("carrier is required");
    }
    hubURA = AdminSupport.hubURA(hubURA);
    reason = AdminSupport.optionalReason(reason);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>(carrier.toObject());
    out.put("hub_ura", hubURA);
    AdminSupport.putOptional(out, "reason", reason);
    return JsonValueWriter.object(out);
  }
}
