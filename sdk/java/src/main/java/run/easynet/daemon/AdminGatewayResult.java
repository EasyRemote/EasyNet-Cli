package run.easynet.daemon;

import java.util.Map;

public record AdminGatewayResult(
    String profile,
    String kind,
    String operation,
    String state,
    String agentURA,
    String deviceURA,
    Boolean ack,
    boolean runtimeNotReady,
    boolean runtimeCatalogNotReady,
    Object nextCursor,
    Map<String, Object> metadata) {
  public AdminGatewayResult {
    if (!AdminSupport.PROFILE.equals(profile)) {
      throw AdminSupport.invalid("invalid admin result projection");
    }
    kind = AdminSupport.required(kind, "kind");
    state = AdminSupport.required(state, "state");
    operation = operation == null ? "" : operation;
    agentURA = agentURA == null ? "" : agentURA;
    deviceURA = deviceURA == null ? "" : deviceURA;
    metadata = AdminSupport.copyObject(metadata);
    if (metadata.isEmpty()) {
      throw AdminSupport.invalid("metadata must be an object");
    }
  }

  public static AdminGatewayResult fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "admin result JSON");
    return new AdminGatewayResult(
        AdminSupport.requiredString(fields, "profile"),
        AdminSupport.requiredString(fields, "kind"),
        AdminSupport.optionalString(fields, "operation"),
        AdminSupport.requiredString(fields, "state"),
        AdminSupport.optionalString(fields, "agent_ura"),
        AdminSupport.optionalString(fields, "device_ura"),
        AdminSupport.optionalBoolean(fields, "ack"),
        fields.get("runtime_not_ready") instanceof Boolean value && value,
        fields.get("runtime_catalog_not_ready") instanceof Boolean value && value,
        fields.get("next_cursor"),
        AdminSupport.requiredObject(fields, "metadata"));
  }
}
