package run.easynet.daemon;

import java.util.Map;

public record MissionEvent(
    String profile,
    String kind,
    String missionID,
    long sequence,
    String eventType,
    long occurredUnixMS,
    boolean terminal,
    Object payload,
    Map<String, Object> receipt,
    Map<String, Object> metadata) {
  public MissionEvent {
    if (!MissionSupport.PROFILE.equals(profile) || !"mission_event".equals(kind)) {
      throw MissionSupport.invalid("invalid mission event projection");
    }
    missionID = MissionSupport.missionID(missionID);
    eventType = MissionSupport.required(eventType, "event_type");
    receipt = MissionSupport.copyObject(receipt);
    metadata = MissionSupport.copyObject(metadata);
  }

  static MissionEvent fromObject(Map<String, Object> fields) {
    return new MissionEvent(
        MissionSupport.requiredString(fields, "profile"),
        MissionSupport.requiredString(fields, "kind"),
        MissionSupport.requiredString(fields, "mission_id"),
        MissionSupport.requiredLong(fields, "sequence"),
        MissionSupport.requiredString(fields, "event_type"),
        MissionSupport.requiredLong(fields, "occurred_unix_ms"),
        MissionSupport.requiredBoolean(fields, "terminal"),
        fields.get("payload"),
        MissionSupport.requiredObject(fields, "receipt"),
        MissionSupport.requiredObject(fields, "metadata"));
  }

  public static MissionEvent fromJSON(byte[] raw) {
    return fromObject(JsonValueReader.object(raw, "mission event JSON"));
  }
}
