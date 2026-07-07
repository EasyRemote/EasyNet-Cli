package run.easynet.daemon;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

public record PairingPreflight(
    String profile,
    String kind,
    String state,
    String hubURA,
    String deviceURA,
    boolean pairingRequired,
    boolean trustReady,
    List<String> scopes,
    Map<String, Object> metadata) {
  public PairingPreflight {
    if (!AdminSupport.PROFILE.equals(profile) || kind == null || kind.isEmpty()) {
      throw AdminSupport.invalid("invalid pairing preflight projection");
    }
    state = AdminSupport.required(state, "state");
    hubURA = AdminSupport.hubURA(hubURA);
    deviceURA = AdminSupport.deviceURA(deviceURA);
    scopes = scopes == null ? List.of() : List.copyOf(scopes);
    metadata = AdminSupport.copyObject(metadata);
    if (metadata.isEmpty()) {
      throw AdminSupport.invalid("metadata must be an object");
    }
  }

  public static PairingPreflight fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "pairing preflight JSON");
    ArrayList<String> scopes = new ArrayList<>();
    for (Object item : AdminSupport.requiredList(fields, "scopes")) {
      if (!(item instanceof String scope)) {
        throw AdminSupport.invalid("scopes entries must be strings");
      }
      scopes.add(scope);
    }
    return new PairingPreflight(
        AdminSupport.requiredString(fields, "profile"),
        AdminSupport.requiredString(fields, "kind"),
        AdminSupport.requiredString(fields, "state"),
        AdminSupport.requiredString(fields, "hub_ura"),
        AdminSupport.requiredString(fields, "device_ura"),
        AdminSupport.requiredBoolean(fields, "pairing_required"),
        AdminSupport.requiredBoolean(fields, "trust_ready"),
        scopes,
        AdminSupport.requiredObject(fields, "metadata"));
  }
}
