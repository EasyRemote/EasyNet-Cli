package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record HostStreamReadiness(
    String state,
    boolean checked,
    Boolean endpointReady,
    Map<String, Object> metadata) {
  public HostStreamReadiness {
    state = HostBindingSupport.optional(state, "state");
    metadata = HostBindingSupport.copyObject(metadata);
  }

  public static HostStreamReadiness fromMap(Map<String, Object> value) {
    return new HostStreamReadiness(
        HostBindingSupport.optionalString(value.get("state"), "state"),
        value.get("checked") instanceof Boolean checked && checked,
        value.get("endpoint_ready") instanceof Boolean endpointReady ? endpointReady : null,
        without(value, "state", "checked", "endpoint_ready"));
  }

  public Map<String, Object> toObject() {
    LinkedHashMap<String, Object> object = new LinkedHashMap<>(metadata);
    if (!state.isEmpty()) {
      object.put("state", state);
    }
    object.put("checked", checked);
    object.put("endpoint_ready", endpointReady);
    return object;
  }

  private static Map<String, Object> without(Map<String, Object> value, String... keys) {
    LinkedHashMap<String, Object> object = new LinkedHashMap<>(value);
    for (String key : keys) {
      object.remove(key);
    }
    return object;
  }
}
