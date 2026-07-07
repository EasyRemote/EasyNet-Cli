package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record EventFilter(
    String realm,
    String ownerURA,
    String deviceURA,
    String agentURA,
    String sessionID,
    String invocationID) {
  public EventFilter {
    realm = EventsSupport.optionalNoWhitespace(realm, "realm");
    ownerURA = EventsSupport.optionalClean(ownerURA, "owner_ura");
    deviceURA = EventsSupport.optionalClean(deviceURA, "device_ura");
    agentURA = EventsSupport.optionalClean(agentURA, "agent_ura");
    sessionID = EventsSupport.optionalNoWhitespace(sessionID, "session_id");
    invocationID = EventsSupport.optionalNoWhitespace(invocationID, "invocation_id");
  }

  static EventFilter normalize(EventFilter filter, Map<String, String> topLevel) {
    EventFilter base = filter == null ? new EventFilter(null, null, null, null, null, null) : filter;
    return new EventFilter(
        choose("realm", base.realm, topLevel.get("realm")),
        choose("owner_ura", base.ownerURA, topLevel.get("owner_ura")),
        choose("device_ura", base.deviceURA, topLevel.get("device_ura")),
        choose("agent_ura", base.agentURA, topLevel.get("agent_ura")),
        choose("session_id", base.sessionID, topLevel.get("session_id")),
        choose("invocation_id", base.invocationID, topLevel.get("invocation_id")));
  }

  Map<String, Object> toObject() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    EventsSupport.putOptional(out, "realm", realm);
    EventsSupport.putOptional(out, "owner_ura", ownerURA);
    EventsSupport.putOptional(out, "device_ura", deviceURA);
    EventsSupport.putOptional(out, "agent_ura", agentURA);
    EventsSupport.putOptional(out, "session_id", sessionID);
    EventsSupport.putOptional(out, "invocation_id", invocationID);
    return out;
  }

  private static String choose(String name, String fromFilter, String topLevel) {
    String top = EventsSupport.optionalClean(topLevel, name);
    String filtered = EventsSupport.optionalClean(fromFilter, "filter." + name);
    if (top != null && filtered != null && !top.equals(filtered)) {
      throw EventsSupport.invalid(name + " conflicts with filter field");
    }
    return filtered != null ? filtered : top;
  }
}
