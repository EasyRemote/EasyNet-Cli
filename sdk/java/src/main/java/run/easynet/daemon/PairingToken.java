package run.easynet.daemon;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

public record PairingToken(
    String profile,
    String kind,
    String tokenID,
    String token,
    String hubURA,
    String deviceURA,
    String state,
    long expiresUnixMS,
    List<String> scopes,
    Map<String, Object> metadata) {
  public PairingToken {
    if (!AdminSupport.PROFILE.equals(profile) || kind == null || kind.isEmpty()) {
      throw AdminSupport.invalid("invalid pairing token projection");
    }
    tokenID = AdminSupport.required(tokenID, "token_id");
    token = AdminSupport.required(token, "token");
    hubURA = AdminSupport.hubURA(hubURA);
    deviceURA = AdminSupport.deviceURA(deviceURA);
    state = AdminSupport.required(state, "state");
    if (expiresUnixMS <= 0) {
      throw AdminSupport.invalid("expires_unix_ms is required");
    }
    scopes = scopes == null ? List.of() : List.copyOf(scopes);
    metadata = AdminSupport.copyObject(metadata);
    if (metadata.isEmpty()) {
      throw AdminSupport.invalid("metadata must be an object");
    }
  }

  public static PairingToken fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "pairing token JSON");
    return new PairingToken(
        AdminSupport.requiredString(fields, "profile"),
        AdminSupport.requiredString(fields, "kind"),
        AdminSupport.requiredString(fields, "token_id"),
        AdminSupport.requiredString(fields, "token"),
        AdminSupport.requiredString(fields, "hub_ura"),
        AdminSupport.requiredString(fields, "device_ura"),
        AdminSupport.requiredString(fields, "state"),
        AdminSupport.requiredLong(fields, "expires_unix_ms"),
        strings(fields, "scopes"),
        AdminSupport.requiredObject(fields, "metadata"));
  }

  private static List<String> strings(Map<String, Object> fields, String name) {
    ArrayList<String> out = new ArrayList<>();
    for (Object item : AdminSupport.requiredList(fields, name)) {
      if (!(item instanceof String value)) {
        throw AdminSupport.invalid(name + " entries must be strings");
      }
      out.add(value);
    }
    return out;
  }
}
