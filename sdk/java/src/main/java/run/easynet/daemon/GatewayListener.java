package run.easynet.daemon;

import java.util.Map;

public record GatewayListener(String kind, String endpoint, boolean ready, boolean isPublic) {
  public GatewayListener {
    kind = AdminSupport.required(kind, "kind");
    endpoint = AdminSupport.required(endpoint, "endpoint");
  }

  static GatewayListener fromObject(Map<String, Object> fields) {
    return new GatewayListener(
        AdminSupport.requiredString(fields, "kind"),
        AdminSupport.requiredString(fields, "endpoint"),
        AdminSupport.requiredBoolean(fields, "ready"),
        AdminSupport.requiredBoolean(fields, "public"));
  }
}
