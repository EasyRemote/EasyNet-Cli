package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record HostStreamBindingRequest(
    String bindingID,
    String descriptorRef,
    String endpoint,
    String frameSchema,
    Map<String, Object> cleanup,
    Long timeoutMS,
    Map<String, Object> readiness,
    Map<String, Object> metadata) {
  public HostStreamBindingRequest {
    bindingID = HostBindingSupport.required(bindingID, "binding_id");
    descriptorRef = HostBindingSupport.required(descriptorRef, "descriptor_ref");
    endpoint = HostBindingSupport.endpoint(endpoint);
    frameSchema = HostBindingSupport.frameSchema(frameSchema);
    cleanup = HostBindingSupport.copyObject(cleanup);
    if (timeoutMS != null && timeoutMS < 0) {
      throw HostBindingSupport.invalid("timeout_ms must be non-negative or null");
    }
    readiness = HostBindingSupport.copyObject(readiness);
    metadata = HostBindingSupport.copyObject(metadata);
  }

  public byte[] toJSON() {
    return JsonValueWriter.object(toObject());
  }

  Map<String, Object> toObject() {
    LinkedHashMap<String, Object> object = new LinkedHashMap<>();
    object.put("binding_id", bindingID);
    object.put("descriptor_ref", descriptorRef);
    object.put("endpoint", endpoint);
    object.put("frame_schema", frameSchema);
    if (!cleanup.isEmpty()) {
      object.put("cleanup", cleanup);
    }
    if (timeoutMS != null) {
      object.put("timeout_ms", timeoutMS);
    }
    if (!readiness.isEmpty()) {
      object.put("readiness", readiness);
    }
    if (!metadata.isEmpty()) {
      object.put("metadata", metadata);
    }
    return object;
  }
}
