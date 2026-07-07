package run.easynet.daemon;

import java.util.Map;

public record DeviceSession(
    String profile,
    String kind,
    String sessionID,
    String deviceURA,
    String hubURA,
    String state,
    String sessionKind,
    long createdUnixMS,
    long expiresUnixMS,
    Map<String, Object> metadata) {
  public DeviceSession {
    if (!AdminSupport.PROFILE.equals(profile) || kind == null || kind.isEmpty()) {
      throw AdminSupport.invalid("invalid device session projection");
    }
    sessionID = AdminSupport.identifier(sessionID, "session_id");
    deviceURA = AdminSupport.deviceURA(deviceURA);
    hubURA = AdminSupport.hubURA(hubURA);
    state = AdminSupport.required(state, "state");
    sessionKind = AdminSupport.identifier(sessionKind, "session_kind");
    if (createdUnixMS <= 0) {
      throw AdminSupport.invalid("created_unix_ms is required");
    }
    metadata = AdminSupport.copyObject(metadata);
    if (metadata.isEmpty()) {
      throw AdminSupport.invalid("metadata must be an object");
    }
  }

  static DeviceSession fromObject(Map<String, Object> fields) {
    return new DeviceSession(
        AdminSupport.requiredString(fields, "profile"),
        AdminSupport.requiredString(fields, "kind"),
        AdminSupport.requiredString(fields, "session_id"),
        AdminSupport.requiredString(fields, "device_ura"),
        AdminSupport.requiredString(fields, "hub_ura"),
        AdminSupport.requiredString(fields, "state"),
        AdminSupport.requiredString(fields, "session_kind"),
        AdminSupport.requiredLong(fields, "created_unix_ms"),
        AdminSupport.requiredLong(fields, "expires_unix_ms"),
        AdminSupport.requiredObject(fields, "metadata"));
  }

  public static DeviceSession fromJSON(byte[] raw) {
    return fromObject(JsonValueReader.object(raw, "device session JSON"));
  }
}
