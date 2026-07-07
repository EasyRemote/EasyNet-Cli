package run.easynet.daemon;

import java.util.LinkedHashMap;

public record AdminJoinHubRequest(AdminCarrierBase carrier, String hubURA, String deviceURA) {
  public AdminJoinHubRequest {
    if (carrier == null) {
      throw AdminSupport.invalid("carrier is required");
    }
    hubURA = AdminSupport.hubURA(hubURA);
    deviceURA = AdminSupport.deviceURA(deviceURA);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>(carrier.toObject());
    out.put("hub_ura", hubURA);
    out.put("device_ura", deviceURA);
    return JsonValueWriter.object(out);
  }
}
