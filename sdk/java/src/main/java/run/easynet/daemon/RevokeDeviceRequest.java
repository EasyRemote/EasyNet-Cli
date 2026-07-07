package run.easynet.daemon;

import java.util.LinkedHashMap;

public record RevokeDeviceRequest(AdminCarrierBase carrier, String deviceURA, String reason) {
  public RevokeDeviceRequest {
    if (carrier == null) {
      throw AdminSupport.invalid("carrier is required");
    }
    deviceURA = AdminSupport.deviceURA(deviceURA);
    reason = AdminSupport.reason(reason);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>(carrier.toObject());
    out.put("device_ura", deviceURA);
    out.put("reason", reason);
    return JsonValueWriter.object(out);
  }
}
