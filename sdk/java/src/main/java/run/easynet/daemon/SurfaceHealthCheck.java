package run.easynet.daemon;

import java.util.Map;

public record SurfaceHealthCheck(
    String name, String state, boolean ready, String message, int latencyMS, Map<String, Object> metadata) {
  public SurfaceHealthCheck {
    name = SurfaceSupport.cleanRequired(name, "name");
    state = SurfaceSupport.cleanRequired(state, "state");
    message = SurfaceSupport.optionalString(message, "message");
    if (latencyMS < 0) {
      throw SurfaceSupport.invalid("latency_ms must be non-negative");
    }
    metadata = SurfaceSupport.copyObject(metadata);
  }

  static SurfaceHealthCheck fromObject(Map<String, Object> fields) {
    return new SurfaceHealthCheck(
        SurfaceSupport.requiredString(fields, "name"),
        SurfaceSupport.requiredString(fields, "state"),
        SurfaceSupport.requiredBoolean(fields, "ready"),
        SurfaceSupport.optionalString(fields.get("message"), "message"),
        SurfaceSupport.requiredInteger(fields, "latency_ms"),
        SurfaceSupport.requiredObject(fields, "metadata"));
  }
}
