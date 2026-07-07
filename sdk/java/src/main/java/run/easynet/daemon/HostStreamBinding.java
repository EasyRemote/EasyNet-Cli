package run.easynet.daemon;

import java.util.Map;

public record HostStreamBinding(
    String bindingID,
    String descriptorRef,
    String endpoint,
    String frameSchema,
    Map<String, Object> cleanup,
    Long timeoutMS,
    Map<String, Object> readiness,
    Map<String, Object> lifecycle,
    Map<String, Object> metadata) {
  public HostStreamBinding {
    bindingID = HostBindingSupport.required(bindingID, "binding_id");
    descriptorRef = HostBindingSupport.required(descriptorRef, "descriptor_ref");
    endpoint = HostBindingSupport.endpoint(endpoint);
    frameSchema = HostBindingSupport.frameSchema(frameSchema);
    cleanup = HostBindingSupport.copyObject(cleanup);
    if (cleanup.isEmpty()) {
      throw HostBindingSupport.invalid("cleanup must be an object");
    }
    if (timeoutMS != null && timeoutMS < 0) {
      throw HostBindingSupport.invalid("timeout_ms must be non-negative or null");
    }
    readiness = HostBindingSupport.copyObject(readiness);
    lifecycle = HostBindingSupport.copyObject(lifecycle);
    metadata = HostBindingSupport.copyObject(metadata);
    if (readiness.isEmpty() || lifecycle.isEmpty() || metadata.isEmpty()) {
      throw HostBindingSupport.invalid("invalid host stream binding projection");
    }
  }

  public static HostStreamBinding fromJSON(byte[] raw) {
    var fields = JsonValueReader.object(raw, "host stream binding JSON");
    return new HostStreamBinding(
        HostBindingSupport.requiredString(fields, "binding_id"),
        HostBindingSupport.requiredString(fields, "descriptor_ref"),
        HostBindingSupport.requiredString(fields, "endpoint"),
        HostBindingSupport.requiredString(fields, "frame_schema"),
        HostBindingSupport.requiredObject(fields, "cleanup"),
        HostBindingSupport.optionalLong(fields.get("timeout_ms"), "timeout_ms"),
        HostBindingSupport.requiredObject(fields, "readiness"),
        HostBindingSupport.requiredObject(fields, "lifecycle"),
        HostBindingSupport.requiredObject(fields, "metadata"));
  }
}
