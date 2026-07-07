package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record EventProjectionInput(
    EventCursor cursor,
    Map<String, Object> event,
    String eventID,
    String resumeToken,
    Object tenantRef) {
  public EventProjectionInput {
    if (cursor == null) {
      throw EventsSupport.invalid("cursor is required");
    }
    event = EventsSupport.requiredCopiedObject(event, "event");
    eventID = EventsSupport.optionalClean(eventID, "event_id");
    resumeToken = EventsSupport.optionalClean(resumeToken, "resume_token");
  }

  byte[] toJSON(String expectedStream) {
    if (expectedStream != null && !expectedStream.isEmpty() && !cursor.stream().equals(expectedStream)) {
      throw EventsSupport.invalid("event cursor stream mismatch");
    }
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("cursor", cursor.toObject(false));
    out.put("event", event);
    EventsSupport.putOptional(out, "event_id", eventID);
    EventsSupport.putOptional(out, "resume_token", resumeToken);
    if (tenantRef != null) {
      out.put("tenant_ref", tenantRef);
    }
    return JsonValueWriter.object(out);
  }
}
