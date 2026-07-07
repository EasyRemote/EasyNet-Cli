package run.easynet.daemon;

import java.util.Map;

public record DeviceCredentialVerification(
    String profile,
    String kind,
    boolean verified,
    String credentialID,
    String deviceURA,
    String hubURA,
    String method,
    Map<String, Object> metadata) {
  public DeviceCredentialVerification {
    if (!AdminSupport.PROFILE.equals(profile) || kind == null || kind.isEmpty()) {
      throw AdminSupport.invalid("invalid device credential verification projection");
    }
    credentialID = AdminSupport.identifier(credentialID, "credential_id");
    deviceURA = AdminSupport.deviceURA(deviceURA);
    hubURA = AdminSupport.hubURA(hubURA);
    method = AdminSupport.required(method, "method");
    metadata = AdminSupport.copyObject(metadata);
    if (metadata.isEmpty()) {
      throw AdminSupport.invalid("metadata must be an object");
    }
  }

  public static DeviceCredentialVerification fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "device credential verification JSON");
    return new DeviceCredentialVerification(
        AdminSupport.requiredString(fields, "profile"),
        AdminSupport.requiredString(fields, "kind"),
        AdminSupport.requiredBoolean(fields, "verified"),
        AdminSupport.requiredString(fields, "credential_id"),
        AdminSupport.requiredString(fields, "device_ura"),
        AdminSupport.requiredString(fields, "hub_ura"),
        AdminSupport.requiredString(fields, "method"),
        AdminSupport.requiredObject(fields, "metadata"));
  }
}
