package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record EventDropReportInput(
    EventCursor cursor,
    long occurredUnixMS,
    int droppedCount,
    Integer reconnectAfterMS,
    String reason,
    String eventID,
    String resumeToken,
    Object tenantRef) {
  public EventDropReportInput {
    if (cursor == null || !cursor.stream().equals("directory")) {
      throw EventsSupport.invalid("event cursor stream mismatch");
    }
    if (occurredUnixMS < 0) {
      throw EventsSupport.invalid("occurred_unix_ms must be non-negative");
    }
    if (droppedCount <= 0) {
      throw EventsSupport.invalid("dropped_count must be greater than zero");
    }
    if (reconnectAfterMS != null && reconnectAfterMS < 0) {
      throw EventsSupport.invalid("reconnect_after_ms must be non-negative");
    }
    reason = EventsSupport.optionalClean(reason, "reason");
    eventID = EventsSupport.optionalClean(eventID, "event_id");
    resumeToken = EventsSupport.optionalClean(resumeToken, "resume_token");
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("cursor", cursor.toObject(false));
    out.put("occurred_unix_ms", occurredUnixMS);
    out.put("dropped_count", droppedCount);
    if (reconnectAfterMS != null) {
      out.put("reconnect_after_ms", reconnectAfterMS);
    }
    EventsSupport.putOptional(out, "reason", reason);
    EventsSupport.putOptional(out, "event_id", eventID);
    EventsSupport.putOptional(out, "resume_token", resumeToken);
    if (tenantRef != null) {
      out.put("tenant_ref", tenantRef);
    }
    return JsonValueWriter.object(out);
  }
}
