package run.easynet.daemon;

import java.util.Map;

public record EventFrame(
    String profile,
    String stream,
    String kind,
    String eventID,
    EventCursor cursor,
    String resumeToken,
    long occurredUnixMS,
    String occurredAt,
    Object subjectRef,
    Object tenantRef,
    Object payload,
    int droppedCount,
    Integer reconnectAfterMS,
    boolean terminal,
    Map<String, Object> metadata) {
  public EventFrame {
    profile = EventsSupport.cleanRequired(profile, "profile");
    stream = EventsSupport.requiredStream(stream, "stream");
    kind = EventsSupport.cleanRequired(kind, "kind");
    eventID = EventsSupport.cleanRequired(eventID, "event_id");
    if (!profile.equals(EventsSupport.PROFILE)) {
      throw EventsSupport.invalid("invalid event frame projection");
    }
    if (cursor == null || !cursor.stream().equals(stream)) {
      throw EventsSupport.invalid("event cursor stream mismatch");
    }
    resumeToken = EventsSupport.cleanRequired(resumeToken, "resume_token");
    if (occurredUnixMS < 0) {
      throw EventsSupport.invalid("occurred_unix_ms must be non-negative");
    }
    occurredAt = EventsSupport.cleanRequired(occurredAt, "occurred_at");
    if (droppedCount < 0) {
      throw EventsSupport.invalid("dropped_count must be non-negative");
    }
    if (reconnectAfterMS != null && reconnectAfterMS < 0) {
      throw EventsSupport.invalid("reconnect_after_ms must be non-negative");
    }
    if (kind.contains("drop_report") && droppedCount == 0) {
      throw EventsSupport.invalid("dropped_count must be greater than zero");
    }
    if (kind.contains("terminal") && !terminal) {
      throw EventsSupport.invalid("terminal event frame must be terminal");
    }
    metadata = EventsSupport.copyObject(metadata);
  }

  public static EventFrame fromJSON(byte[] raw) {
    return fromObject(JsonValueReader.object(raw, "event frame JSON"));
  }

  static EventFrame fromObject(Map<String, Object> fields) {
    return new EventFrame(
        EventsSupport.requiredString(fields, "profile"),
        EventsSupport.requiredString(fields, "stream"),
        EventsSupport.requiredString(fields, "kind"),
        EventsSupport.requiredString(fields, "event_id"),
        EventCursor.fromObject(EventsSupport.requiredObject(fields, "cursor"), true),
        EventsSupport.requiredString(fields, "resume_token"),
        EventsSupport.requiredLong(fields, "occurred_unix_ms"),
        EventsSupport.requiredString(fields, "occurred_at"),
        fields.get("subject_ref"),
        fields.get("tenant_ref"),
        fields.get("payload"),
        EventsSupport.requiredInteger(fields, "dropped_count"),
        EventsSupport.optionalInteger(fields.get("reconnect_after_ms"), "reconnect_after_ms"),
        EventsSupport.requiredBoolean(fields, "terminal"),
        EventsSupport.requiredObject(fields, "metadata"));
  }
}
