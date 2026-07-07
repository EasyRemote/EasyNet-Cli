package run.easynet.daemon;

import java.util.List;
import java.util.Map;
import java.util.Objects;

public record DirectoryPage(
    String profile,
    String kind,
    String itemKind,
    List<Object> items,
    String nextCursor,
    int limit,
    String source,
    Map<String, Object> metadata) {
  public DirectoryPage {
    if (!"directory_identity".equals(profile)) {
      throw DirectoryIdentitySupport.invalidField("profile", "must be directory_identity");
    }
    if (!kind.equals("device_page") && !kind.equals("agent_page") && !kind.equals("ability_page")) {
      throw DirectoryIdentitySupport.invalidField("kind", "must be a directory page kind");
    }
    if (!itemKind.equals("device") && !itemKind.equals("agent") && !itemKind.equals("ability")) {
      throw DirectoryIdentitySupport.invalidField("item_kind", "must be device, agent, or ability");
    }
    if (!"read_model".equals(source)) {
      throw DirectoryIdentitySupport.invalidField("source", "must be read_model");
    }
    DirectoryIdentitySupport.normalizeLimit(limit);
    items = DirectoryIdentitySupport.copyList(items);
    metadata = DirectoryIdentitySupport.requiredCopiedObject(metadata, "metadata");
  }

  public static DirectoryPage fromJSON(byte[] raw) {
    Objects.requireNonNull(raw, "raw");
    Map<String, Object> fields = JsonValueReader.object(raw, "directory page JSON");
    return new DirectoryPage(
        DirectoryIdentitySupport.requiredString(fields, "profile"),
        DirectoryIdentitySupport.requiredString(fields, "kind"),
        DirectoryIdentitySupport.requiredString(fields, "item_kind"),
        DirectoryIdentitySupport.requiredList(fields, "items"),
        DirectoryIdentitySupport.optionalString(fields.get("next_cursor"), "next_cursor"),
        DirectoryIdentitySupport.requiredInteger(fields, "limit"),
        DirectoryIdentitySupport.requiredString(fields, "source"),
        DirectoryIdentitySupport.requiredObject(fields, "metadata"));
  }
}
