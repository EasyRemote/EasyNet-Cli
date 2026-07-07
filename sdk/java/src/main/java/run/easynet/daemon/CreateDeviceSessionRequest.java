package run.easynet.daemon;

import java.util.LinkedHashMap;

public record CreateDeviceSessionRequest(
    AdminCarrierBase carrier, String deviceURA, String hubURA, String sessionKind, long expiresUnixMS) {
  public CreateDeviceSessionRequest {
    if (carrier == null) {
      throw AdminSupport.invalid("carrier is required");
    }
    deviceURA = AdminSupport.deviceURA(deviceURA);
    hubURA = AdminSupport.hubURA(hubURA);
    sessionKind = AdminSupport.identifier(sessionKind, "session_kind");
    if (expiresUnixMS < 0) {
      throw AdminSupport.invalid("expires_unix_ms must be non-negative");
    }
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>(carrier.toObject());
    out.put("device_ura", deviceURA);
    out.put("hub_ura", hubURA);
    out.put("session_kind", sessionKind);
    if (expiresUnixMS > 0) {
      out.put("expires_unix_ms", expiresUnixMS);
    }
    return JsonValueWriter.object(out);
  }
}
