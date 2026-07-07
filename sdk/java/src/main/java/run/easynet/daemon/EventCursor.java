package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record EventCursor(String stream, int sequence, String token) {
  public EventCursor {
    stream = EventsSupport.requiredStream(stream, "stream");
    if (sequence < 0) {
      throw EventsSupport.invalid("sequence must be a non-negative integer");
    }
    token = token == null || token.isEmpty() ? stream + ":" + sequence : EventsSupport.cleanRequired(token, "token");
    if (!token.equals(stream + ":" + sequence)) {
      throw EventsSupport.invalid("event cursor token must match stream sequence");
    }
  }

  public static EventCursor fromObject(Map<String, Object> fields, boolean requireToken) {
    return new EventCursor(
        EventsSupport.requiredString(fields, "stream"),
        EventsSupport.requiredInteger(fields, "sequence"),
        requireToken ? EventsSupport.requiredString(fields, "token") : EventsSupport.optionalString(fields.get("token"), "token"));
  }

  public Map<String, Object> toObject(boolean includeToken) {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("stream", stream);
    out.put("sequence", sequence);
    if (includeToken) {
      out.put("token", token);
    }
    return out;
  }

  public String resumeToken() {
    return token;
  }
}
