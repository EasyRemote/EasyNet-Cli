package run.easynet.daemon;

import java.util.Map;

public final class WrapperBrowserSession extends WrapperSessionRecord {
  public WrapperBrowserSession(
      String profile, String kind, String sessionID, String ownerURA, String state, String browserRef, Map<String, Object> metadata) {
    super(profile, kind, "browser_session", sessionID, ownerURA, state, "browser_ref", browserRef, metadata);
  }

  public static WrapperBrowserSession fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "wrapper browser session JSON");
    return new WrapperBrowserSession(
        WrapperSupport.requiredString(fields.get("profile"), "profile"),
        WrapperSupport.requiredString(fields.get("kind"), "kind"),
        WrapperSupport.requiredString(fields.get("session_id"), "session_id"),
        WrapperSupport.requiredString(fields.get("owner_ura"), "owner_ura"),
        WrapperSupport.requiredString(fields.get("state"), "state"),
        WrapperSupport.optionalString(fields.get("browser_ref"), "browser_ref"),
        WrapperSupport.requiredObject(fields.get("metadata"), "metadata"));
  }

  public String sessionID() {
    return sessionID;
  }

  public String browserRef() {
    return refValue;
  }

  public byte[] toJSON() {
    return JsonValueWriter.object(toObject());
  }
}
