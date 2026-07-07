package run.easynet.daemon;

import java.util.LinkedHashMap;

public record VerifyDeviceCredentialRequest(
    AdminCarrierBase carrier, String credentialID, String deviceURA, String hubURA) {
  public VerifyDeviceCredentialRequest {
    if (carrier == null) {
      throw AdminSupport.invalid("carrier is required");
    }
    credentialID = AdminSupport.identifier(credentialID, "credential_id");
    deviceURA = AdminSupport.deviceURA(deviceURA);
    hubURA = AdminSupport.hubURA(hubURA);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>(carrier.toObject());
    out.put("credential_id", credentialID);
    out.put("device_ura", deviceURA);
    out.put("hub_ura", hubURA);
    return JsonValueWriter.object(out);
  }
}
