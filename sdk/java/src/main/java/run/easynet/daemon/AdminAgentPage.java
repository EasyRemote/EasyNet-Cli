package run.easynet.daemon;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

public record AdminAgentPage(
    String profile,
    String kind,
    String state,
    List<AdminAgentRecord> items,
    Object nextCursor,
    Map<String, Object> metadata) {
  public AdminAgentPage {
    if (!AdminSupport.PROFILE.equals(profile) || !"agent_records".equals(kind)) {
      throw AdminSupport.invalid("invalid admin agent page projection");
    }
    state = AdminSupport.required(state, "state");
    items = items == null ? List.of() : List.copyOf(items);
    metadata = AdminSupport.copyObject(metadata);
    if (metadata.isEmpty()) {
      throw AdminSupport.invalid("metadata must be an object");
    }
  }

  public static AdminAgentPage fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "admin agent page JSON");
    ArrayList<AdminAgentRecord> items = new ArrayList<>();
    for (Object item : AdminSupport.requiredList(fields, "items")) {
      Map<String, Object> agent = AdminSupport.optionalObject(item, "items");
      if (agent == null) {
        throw AdminSupport.invalid("items entry must be an object");
      }
      items.add(AdminAgentRecord.fromObject(agent));
    }
    return new AdminAgentPage(
        AdminSupport.requiredString(fields, "profile"),
        AdminSupport.requiredString(fields, "kind"),
        AdminSupport.requiredString(fields, "state"),
        items,
        fields.get("next_cursor"),
        AdminSupport.requiredObject(fields, "metadata"));
  }
}
