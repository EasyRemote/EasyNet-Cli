package run.easynet.daemon;

import java.util.List;
import java.util.Map;

public record AdminAgentRecord(
    String name,
    String agentURA,
    String ownerURA,
    String deviceURA,
    String state,
    String runtime,
    String model,
    String label,
    List<Object> abilities,
    Map<String, Object> metadata) {
  public AdminAgentRecord {
    name = AdminSupport.required(name, "name");
    state = AdminSupport.required(state, "state");
    runtime = AdminSupport.required(runtime, "runtime");
    agentURA = agentURA == null ? "" : agentURA;
    ownerURA = ownerURA == null ? "" : ownerURA;
    deviceURA = deviceURA == null ? "" : deviceURA;
    model = model == null ? "" : model;
    label = label == null ? "" : label;
    abilities = abilities == null ? List.of() : List.copyOf(abilities);
    metadata = AdminSupport.copyObject(metadata);
    if (metadata.isEmpty()) {
      throw AdminSupport.invalid("agent metadata must be an object");
    }
  }

  static AdminAgentRecord fromObject(Map<String, Object> fields) {
    return new AdminAgentRecord(
        AdminSupport.requiredString(fields, "name"),
        AdminSupport.optionalString(fields, "agent_ura"),
        AdminSupport.optionalString(fields, "owner_ura"),
        AdminSupport.optionalString(fields, "device_ura"),
        AdminSupport.requiredString(fields, "state"),
        AdminSupport.requiredString(fields, "runtime"),
        AdminSupport.optionalString(fields, "model"),
        AdminSupport.optionalString(fields, "label"),
        AdminSupport.requiredList(fields, "abilities"),
        AdminSupport.requiredObject(fields, "metadata"));
  }
}
