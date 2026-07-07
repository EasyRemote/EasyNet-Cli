package run.easynet.daemon;

import java.util.Map;

public final class WrapperTerminalSession extends WrapperSessionRecord {
  public WrapperTerminalSession(
      String profile, String kind, String sessionID, String ownerURA, String state, String terminalRef, Map<String, Object> metadata) {
    super(profile, kind, "terminal_session", sessionID, ownerURA, state, "terminal_ref", terminalRef, metadata);
  }

  public static WrapperTerminalSession fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "wrapper terminal session JSON");
    return new WrapperTerminalSession(
        WrapperSupport.requiredString(fields.get("profile"), "profile"),
        WrapperSupport.requiredString(fields.get("kind"), "kind"),
        WrapperSupport.requiredString(fields.get("session_id"), "session_id"),
        WrapperSupport.requiredString(fields.get("owner_ura"), "owner_ura"),
        WrapperSupport.requiredString(fields.get("state"), "state"),
        WrapperSupport.optionalString(fields.get("terminal_ref"), "terminal_ref"),
        WrapperSupport.requiredObject(fields.get("metadata"), "metadata"));
  }

  public String sessionID() {
    return sessionID;
  }

  public String state() {
    return state;
  }

  public String terminalRef() {
    return refValue;
  }

  public byte[] toJSON() {
    return JsonValueWriter.object(toObject());
  }
}
