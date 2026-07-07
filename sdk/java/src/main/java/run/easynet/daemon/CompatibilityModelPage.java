package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public record CompatibilityModelPage(
    String profile,
    String kind,
    String object,
    List<CompatibilityModel> data,
    String nextCursor,
    Map<String, Object> metadata) {
  public CompatibilityModelPage {
    CompatibilitySupport.validateKind(profile, kind, "model_page");
    CompatibilitySupport.validateObject(object, "list", "model_page");
    data = List.copyOf(data);
    nextCursor = CompatibilitySupport.optionalString(nextCursor, "next_cursor");
    metadata = CompatibilitySupport.copyObject(metadata);
  }

  public static CompatibilityModelPage fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "compatibility model page JSON");
    List<CompatibilityModel> models = CompatibilitySupport.requiredList(fields.get("data"), "data").stream()
        .map(item -> CompatibilityModel.fromObject(CompatibilitySupport.requiredObject(item, "model")))
        .toList();
    return new CompatibilityModelPage(
        CompatibilitySupport.requiredString(fields.get("profile"), "profile"),
        CompatibilitySupport.requiredString(fields.get("kind"), "kind"),
        CompatibilitySupport.requiredString(fields.get("object"), "object"),
        models,
        CompatibilitySupport.optionalString(fields.get("next_cursor"), "next_cursor"),
        CompatibilitySupport.requiredObject(fields.get("metadata"), "metadata"));
  }

  public byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("profile", profile);
    out.put("kind", kind);
    out.put("object", object);
    out.put("data", data.stream().map(CompatibilityModel::toObject).toList());
    out.put("next_cursor", nextCursor);
    out.put("metadata", metadata);
    return JsonValueWriter.object(out);
  }
}
