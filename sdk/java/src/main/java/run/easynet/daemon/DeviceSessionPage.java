package run.easynet.daemon;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

public record DeviceSessionPage(
    String profile,
    String kind,
    String state,
    List<DeviceSession> items,
    Object nextCursor,
    Map<String, Object> metadata) {
  public DeviceSessionPage {
    if (!AdminSupport.PROFILE.equals(profile) || !"device_sessions".equals(kind)) {
      throw AdminSupport.invalid("invalid device session page projection");
    }
    state = AdminSupport.required(state, "state");
    items = items == null ? List.of() : List.copyOf(items);
    metadata = AdminSupport.copyObject(metadata);
    if (metadata.isEmpty()) {
      throw AdminSupport.invalid("metadata must be an object");
    }
  }

  public static DeviceSessionPage fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "device session page JSON");
    ArrayList<DeviceSession> items = new ArrayList<>();
    for (Object item : AdminSupport.requiredList(fields, "items")) {
      Map<String, Object> session = AdminSupport.optionalObject(item, "items");
      if (session == null) {
        throw AdminSupport.invalid("items entry must be an object");
      }
      items.add(DeviceSession.fromObject(session));
    }
    return new DeviceSessionPage(
        AdminSupport.requiredString(fields, "profile"),
        AdminSupport.requiredString(fields, "kind"),
        AdminSupport.requiredString(fields, "state"),
        items,
        fields.get("next_cursor"),
        AdminSupport.requiredObject(fields, "metadata"));
  }
}
