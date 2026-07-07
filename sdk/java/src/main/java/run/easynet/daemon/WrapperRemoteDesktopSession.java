package run.easynet.daemon;

import java.util.Map;

public final class WrapperRemoteDesktopSession extends WrapperSessionRecord {
  public WrapperRemoteDesktopSession(
      String profile, String kind, String sessionID, String ownerURA, String state, String displayRef, Map<String, Object> metadata) {
    super(profile, kind, "remote_desktop_session", sessionID, ownerURA, state, "display_ref", displayRef, metadata);
  }

  public static WrapperRemoteDesktopSession fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "wrapper remote desktop session JSON");
    return new WrapperRemoteDesktopSession(
        WrapperSupport.requiredString(fields.get("profile"), "profile"),
        WrapperSupport.requiredString(fields.get("kind"), "kind"),
        WrapperSupport.requiredString(fields.get("session_id"), "session_id"),
        WrapperSupport.requiredString(fields.get("owner_ura"), "owner_ura"),
        WrapperSupport.requiredString(fields.get("state"), "state"),
        WrapperSupport.optionalString(fields.get("display_ref"), "display_ref"),
        WrapperSupport.requiredObject(fields.get("metadata"), "metadata"));
  }

  public String sessionID() {
    return sessionID;
  }

  public String displayRef() {
    return refValue;
  }

  public byte[] toJSON() {
    return JsonValueWriter.object(toObject());
  }
}
