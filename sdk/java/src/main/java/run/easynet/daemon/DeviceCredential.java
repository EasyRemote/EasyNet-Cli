package run.easynet.daemon;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

public record DeviceCredential(
    String profile,
    String kind,
    String credentialID,
    String deviceURA,
    String hubURA,
    String state,
    long issuedUnixMS,
    long expiresUnixMS,
    List<String> scopes,
    Map<String, Object> metadata) {
  public DeviceCredential {
    if (!AdminSupport.PROFILE.equals(profile) || kind == null || kind.isEmpty()) {
      throw AdminSupport.invalid("invalid device credential projection");
    }
    credentialID = AdminSupport.required(credentialID, "credential_id");
    deviceURA = AdminSupport.deviceURA(deviceURA);
    hubURA = AdminSupport.hubURA(hubURA);
    state = AdminSupport.required(state, "state");
    if (issuedUnixMS <= 0 || expiresUnixMS <= 0) {
      throw AdminSupport.invalid("credential timestamps are required");
    }
    scopes = scopes == null ? List.of() : List.copyOf(scopes);
    metadata = AdminSupport.copyObject(metadata);
    if (metadata.isEmpty()) {
      throw AdminSupport.invalid("metadata must be an object");
    }
  }

  public static DeviceCredential fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "device credential JSON");
    ArrayList<String> scopes = new ArrayList<>();
    for (Object item : AdminSupport.requiredList(fields, "scopes")) {
      if (!(item instanceof String value)) {
        throw AdminSupport.invalid("scopes entries must be strings");
      }
      scopes.add(value);
    }
    return new DeviceCredential(
        AdminSupport.requiredString(fields, "profile"),
        AdminSupport.requiredString(fields, "kind"),
        AdminSupport.requiredString(fields, "credential_id"),
        AdminSupport.requiredString(fields, "device_ura"),
        AdminSupport.requiredString(fields, "hub_ura"),
        AdminSupport.requiredString(fields, "state"),
        AdminSupport.requiredLong(fields, "issued_unix_ms"),
        AdminSupport.requiredLong(fields, "expires_unix_ms"),
        scopes,
        AdminSupport.requiredObject(fields, "metadata"));
  }
}
