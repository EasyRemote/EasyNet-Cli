package run.easynet.daemon;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

public record GatewayStatus(
    String profile,
    String gatewayID,
    boolean ready,
    String state,
    boolean processLive,
    boolean controlReady,
    boolean runtimeReady,
    boolean directoryReady,
    boolean trustReady,
    boolean publicListenerReady,
    List<GatewayListener> listeners,
    Map<String, Object> identity,
    Map<String, Object> metadata) {
  public GatewayStatus {
    if (!AdminSupport.PROFILE.equals(profile)) {
      throw AdminSupport.invalid("invalid gateway status projection");
    }
    gatewayID = AdminSupport.required(gatewayID, "gateway_id");
    state = AdminSupport.required(state, "state");
    listeners = listeners == null ? List.of() : List.copyOf(listeners);
    identity = AdminSupport.copyObject(identity);
    metadata = AdminSupport.copyObject(metadata);
    if (metadata.isEmpty()) {
      throw AdminSupport.invalid("metadata must be an object");
    }
  }

  public static GatewayStatus fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "gateway status JSON");
    ArrayList<GatewayListener> listeners = new ArrayList<>();
    for (Object item : AdminSupport.requiredList(fields, "listeners")) {
      Map<String, Object> listener = AdminSupport.optionalObject(item, "listeners");
      if (listener == null) {
        throw AdminSupport.invalid("listeners entry must be an object");
      }
      listeners.add(GatewayListener.fromObject(listener));
    }
    return new GatewayStatus(
        AdminSupport.requiredString(fields, "profile"),
        AdminSupport.requiredString(fields, "gateway_id"),
        AdminSupport.requiredBoolean(fields, "ready"),
        AdminSupport.requiredString(fields, "state"),
        AdminSupport.requiredBoolean(fields, "process_live"),
        AdminSupport.requiredBoolean(fields, "control_ready"),
        AdminSupport.requiredBoolean(fields, "runtime_ready"),
        AdminSupport.requiredBoolean(fields, "directory_ready"),
        AdminSupport.requiredBoolean(fields, "trust_ready"),
        AdminSupport.requiredBoolean(fields, "public_listener_ready"),
        listeners,
        AdminSupport.requiredObject(fields, "identity"),
        AdminSupport.requiredObject(fields, "metadata"));
  }
}
