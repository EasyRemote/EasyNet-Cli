package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public final class WrapperMediaSession extends WrapperSessionRecord {
  private final String mediaKind;

  public WrapperMediaSession(
      String profile,
      String kind,
      String sessionID,
      String ownerURA,
      String state,
      String mediaKind,
      String streamRef,
      Map<String, Object> metadata) {
    super(profile, kind, "media_session", sessionID, ownerURA, state, "stream_ref", streamRef, metadata);
    this.mediaKind = WrapperSupport.requiredString(mediaKind, "media_kind");
  }

  public static WrapperMediaSession fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "wrapper media session JSON");
    return new WrapperMediaSession(
        WrapperSupport.requiredString(fields.get("profile"), "profile"),
        WrapperSupport.requiredString(fields.get("kind"), "kind"),
        WrapperSupport.requiredString(fields.get("session_id"), "session_id"),
        WrapperSupport.requiredString(fields.get("owner_ura"), "owner_ura"),
        WrapperSupport.requiredString(fields.get("state"), "state"),
        WrapperSupport.requiredString(fields.get("media_kind"), "media_kind"),
        WrapperSupport.optionalString(fields.get("stream_ref"), "stream_ref"),
        WrapperSupport.requiredObject(fields.get("metadata"), "metadata"));
  }

  public String sessionID() {
    return sessionID;
  }

  public String mediaKind() {
    return mediaKind;
  }

  public String streamRef() {
    return refValue;
  }

  public byte[] toJSON() {
    LinkedHashMap<String, Object> object = new LinkedHashMap<>(toObject());
    object.put("media_kind", mediaKind);
    object.remove("stream_ref");
    object.put("stream_ref", refValue);
    object.put("metadata", metadata);
    return JsonValueWriter.object(object);
  }
}
