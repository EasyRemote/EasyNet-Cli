package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.List;

public record CreatePairingRequest(
    AdminCarrierBase carrier, String hubURA, String deviceURA, long expiresUnixMS, List<String> scopes) {
  public CreatePairingRequest {
    if (carrier == null) {
      throw AdminSupport.invalid("carrier is required");
    }
    hubURA = AdminSupport.hubURA(hubURA);
    deviceURA = AdminSupport.deviceURA(deviceURA);
    if (expiresUnixMS <= 0) {
      throw AdminSupport.invalid("expires_unix_ms is required");
    }
    scopes = AdminSupport.scopes(scopes);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>(carrier.toObject());
    out.put("hub_ura", hubURA);
    out.put("device_ura", deviceURA);
    out.put("expires_unix_ms", expiresUnixMS);
    if (!scopes.isEmpty()) {
      out.put("scopes", scopes);
    }
    return JsonValueWriter.object(out);
  }
}
