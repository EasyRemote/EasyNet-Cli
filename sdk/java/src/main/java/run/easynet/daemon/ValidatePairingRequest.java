package run.easynet.daemon;

import java.util.LinkedHashMap;

public record ValidatePairingRequest(AdminCarrierBase carrier, String token, String deviceURA) {
  public ValidatePairingRequest {
    if (carrier == null) {
      throw AdminSupport.invalid("carrier is required");
    }
    token = AdminSupport.identifier(token, "token");
    deviceURA = AdminSupport.deviceURA(deviceURA);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>(carrier.toObject());
    out.put("token", token);
    out.put("device_ura", deviceURA);
    return JsonValueWriter.object(out);
  }
}
