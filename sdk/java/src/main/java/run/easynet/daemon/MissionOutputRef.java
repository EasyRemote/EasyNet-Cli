package run.easynet.daemon;

import java.util.Map;

public record MissionOutputRef(String kind, String path, Map<String, Object> metadata) {
  public MissionOutputRef {
    kind = MissionSupport.required(kind, "kind");
    path = path == null ? "" : path;
    metadata = MissionSupport.copyObject(metadata);
  }

  static MissionOutputRef fromObject(Map<String, Object> fields) {
    return new MissionOutputRef(
        MissionSupport.requiredString(fields, "kind"),
        MissionSupport.optionalString(fields, "path"),
        MissionSupport.optionalObject(fields.get("metadata"), "metadata"));
  }
}
