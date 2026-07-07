package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record EventsSubscriptionRequest(
    EventsCarrierBase base,
    String stream,
    EventFilter filter,
    String realm,
    String ownerURA,
    String deviceURA,
    String agentURA,
    String sessionID,
    String sessionURA,
    String invocationID,
    EventCursor resumeCursor,
    Integer heartbeatIntervalMS) {
  public EventsSubscriptionRequest {
    if (base == null) {
      throw EventsSupport.invalid("events carrier base is required");
    }
    stream = EventsSupport.optionalClean(stream, "stream");
    realm = EventsSupport.optionalNoWhitespace(realm, "realm");
    ownerURA = EventsSupport.optionalClean(ownerURA, "owner_ura");
    deviceURA = EventsSupport.optionalClean(deviceURA, "device_ura");
    agentURA = EventsSupport.optionalClean(agentURA, "agent_ura");
    sessionID = EventsSupport.optionalNoWhitespace(sessionID, "session_id");
    sessionURA = EventsSupport.optionalClean(sessionURA, "session_ura");
    invocationID = EventsSupport.optionalNoWhitespace(invocationID, "invocation_id");
    if (heartbeatIntervalMS != null
        && (heartbeatIntervalMS < EventsSupport.MIN_HEARTBEAT_MS
            || heartbeatIntervalMS > EventsSupport.MAX_HEARTBEAT_MS)) {
      throw EventsSupport.invalid("heartbeat_interval_ms exceeds bounds");
    }
  }

  byte[] toJSON(String expectedStream) {
    return JsonValueWriter.object(toObject(expectedStream));
  }

  LinkedHashMap<String, Object> toObject(String expectedStream) {
    EventsSupport.requiredStream(expectedStream, "stream");
    String normalizedStream = stream == null ? expectedStream : stream;
    if (!normalizedStream.equals(expectedStream)) {
      throw EventsSupport.invalid("event subscription stream mismatch");
    }
    EventFilter normalized =
        EventFilter.normalize(
            filter,
            Map.of(
                "realm", realm == null ? "" : realm,
                "owner_ura", ownerURA == null ? "" : ownerURA,
                "device_ura", deviceURA == null ? "" : deviceURA,
                "agent_ura", agentURA == null ? "" : agentURA,
                "session_id", sessionID == null ? "" : sessionID,
                "invocation_id", invocationID == null ? "" : invocationID));
    if (resumeCursor != null && !resumeCursor.stream().equals(expectedStream)) {
      throw EventsSupport.invalid("resume cursor stream mismatch");
    }
    if (expectedStream.equals("session")) {
      if (sessionURA != null) {
        throw EventsSupport.invalid("session_ura cannot be converted into daemon session_id");
      }
      if (normalized.sessionID() == null) {
        throw EventsSupport.invalid("session_id is required");
      }
    }
    if (expectedStream.equals("invocation") && normalized.invocationID() == null) {
      throw EventsSupport.invalid("invocation_id is required");
    }

    LinkedHashMap<String, Object> out = base.toObject();
    out.put("stream", normalizedStream);
    Map<String, Object> filterObject = normalized.toObject();
    if (filter != null && !filterObject.isEmpty()) {
      out.put("filter", filterObject);
    }
    EventsSupport.putOptional(out, "realm", normalized.realm());
    EventsSupport.putOptional(out, "owner_ura", normalized.ownerURA());
    EventsSupport.putOptional(out, "device_ura", normalized.deviceURA());
    EventsSupport.putOptional(out, "agent_ura", normalized.agentURA());
    EventsSupport.putOptional(out, "session_id", normalized.sessionID());
    EventsSupport.putOptional(out, "session_ura", sessionURA);
    EventsSupport.putOptional(out, "invocation_id", normalized.invocationID());
    if (resumeCursor != null) {
      out.put("resume_cursor", resumeCursor.toObject(false));
    }
    if (heartbeatIntervalMS != null) {
      out.put("heartbeat_interval_ms", heartbeatIntervalMS);
    }
    return out;
  }
}
