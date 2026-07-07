package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.List;

public record PairingPreflightRequest(
    AdminCarrierBase carrier, String hubURA, String deviceURA, List<String> requestedScopes) {
  public PairingPreflightRequest {
    if (carrier == null) {
      throw AdminSupport.invalid("carrier is required");
    }
    hubURA = AdminSupport.hubURA(hubURA);
    deviceURA = AdminSupport.deviceURA(deviceURA);
    requestedScopes = AdminSupport.scopes(requestedScopes);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>(carrier.toObject());
    out.put("hub_ura", hubURA);
    out.put("device_ura", deviceURA);
    if (!requestedScopes.isEmpty()) {
      out.put("requested_scopes", requestedScopes);
    }
    return JsonValueWriter.object(out);
  }
}
