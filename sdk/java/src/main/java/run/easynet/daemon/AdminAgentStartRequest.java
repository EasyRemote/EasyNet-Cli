package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public record AdminAgentStartRequest(
    AdminCarrierBase carrier,
    String name,
    String agentType,
    Map<String, Object> entry,
    String model,
    String label,
    String command,
    List<String> commandArgs,
    String rootPath,
    Boolean modelPresent,
    Boolean materializeDirectory,
    Boolean updateExistingSpec,
    Boolean projectWorkspace) {
  public AdminAgentStartRequest {
    if (carrier == null) {
      throw AdminSupport.invalid("carrier is required");
    }
    name = AdminSupport.agentName(name);
    agentType = AdminSupport.optional(agentType, "agent_type");
    entry = AdminSupport.copyObject(entry);
    if (agentType.isEmpty() && entry.isEmpty()) {
      throw AdminSupport.invalid("either agent_type or entry is required");
    }
    model = AdminSupport.optional(model, "model");
    label = AdminSupport.optional(label, "label");
    command = AdminSupport.optional(command, "command");
    commandArgs = commandArgs == null ? List.of() : List.copyOf(commandArgs);
    rootPath = AdminSupport.optional(rootPath, "root_path");
    if (!rootPath.isEmpty() && (!rootPath.startsWith("/") || rootPath.contains("/../") || rootPath.endsWith("/.."))) {
      throw AdminSupport.invalid("root_path must be absolute and must not contain parent traversal");
    }
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>(carrier.toObject());
    out.put("name", name);
    AdminSupport.putOptional(out, "agent_type", agentType);
    if (!entry.isEmpty()) {
      out.put("entry", entry);
    }
    AdminSupport.putOptional(out, "model", model);
    AdminSupport.putOptional(out, "label", label);
    AdminSupport.putOptional(out, "command", command);
    if (!commandArgs.isEmpty()) {
      out.put("command_args", commandArgs);
    }
    AdminSupport.putOptional(out, "root_path", rootPath);
    AdminSupport.putOptional(out, "model_present", modelPresent);
    AdminSupport.putOptional(out, "materialize_directory", materializeDirectory);
    AdminSupport.putOptional(out, "update_existing_spec", updateExistingSpec);
    AdminSupport.putOptional(out, "project_workspace", projectWorkspace);
    return JsonValueWriter.object(out);
  }
}
