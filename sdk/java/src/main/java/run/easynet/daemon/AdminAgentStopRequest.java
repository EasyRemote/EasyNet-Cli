package run.easynet.daemon;

import java.util.LinkedHashMap;

public record AdminAgentStopRequest(AdminCarrierBase carrier, String name, String agentURA) {
  public AdminAgentStopRequest {
    if (carrier == null) {
      throw AdminSupport.invalid("carrier is required");
    }
    name = AdminSupport.optionalAgentName(name);
    agentURA = AdminSupport.optionalAgentURA(agentURA);
    if (name.isEmpty() && agentURA.isEmpty()) {
      throw AdminSupport.invalid("either name or agent_ura is required");
    }
    if (!name.isEmpty() && !agentURA.isEmpty() && !agentURA.endsWith("." + name)) {
      throw AdminSupport.invalid("agent_ura must name the same hosted agent as name");
    }
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>(carrier.toObject());
    AdminSupport.putOptional(out, "name", name);
    AdminSupport.putOptional(out, "agent_ura", agentURA);
    return JsonValueWriter.object(out);
  }
}
