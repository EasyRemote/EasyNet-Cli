package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record HostStreamCleanup(String mode, Map<String, Object> metadata) {
  public HostStreamCleanup {
    mode = HostBindingSupport.optional(mode, "mode");
    metadata = HostBindingSupport.copyObject(metadata);
  }

  public static HostStreamCleanup fromMap(Map<String, Object> value) {
    LinkedHashMap<String, Object> metadata = new LinkedHashMap<>(value);
    metadata.remove("mode");
    return new HostStreamCleanup(
        HostBindingSupport.optionalString(value.get("mode"), "mode"), metadata);
  }

  public Map<String, Object> toObject() {
    LinkedHashMap<String, Object> object = new LinkedHashMap<>(metadata);
    if (!mode.isEmpty()) {
      object.put("mode", mode);
    }
    return object;
  }
}
