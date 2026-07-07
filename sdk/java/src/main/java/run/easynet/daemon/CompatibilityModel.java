package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record CompatibilityModel(
    String profile,
    String kind,
    String id,
    String object,
    long created,
    String ownedBy,
    String abilityRef,
    Map<String, Object> metadata) {
  public CompatibilityModel {
    CompatibilitySupport.validateKind(profile, kind, "model");
    CompatibilitySupport.validateObject(object, "model", "model");
    id = CompatibilitySupport.requiredAbilityURA(id, "id");
    if (created < 0) {
      throw CompatibilitySupport.invalid("created must be a non-negative integer");
    }
    ownedBy = CompatibilitySupport.requiredString(ownedBy, "owned_by");
    abilityRef = CompatibilitySupport.requiredAbilityURA(abilityRef, "ability_ref");
    metadata = CompatibilitySupport.copyObject(metadata);
  }

  static CompatibilityModel fromObject(Map<String, Object> fields) {
    return new CompatibilityModel(
        CompatibilitySupport.requiredString(fields.get("profile"), "profile"),
        CompatibilitySupport.requiredString(fields.get("kind"), "kind"),
        CompatibilitySupport.requiredString(fields.get("id"), "id"),
        CompatibilitySupport.requiredString(fields.get("object"), "object"),
        CompatibilitySupport.requiredNonNegativeInteger(fields.get("created"), "created"),
        CompatibilitySupport.requiredString(fields.get("owned_by"), "owned_by"),
        CompatibilitySupport.requiredString(fields.get("ability_ref"), "ability_ref"),
        CompatibilitySupport.requiredObject(fields.get("metadata"), "metadata"));
  }

  Map<String, Object> toObject() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("profile", profile);
    out.put("kind", kind);
    out.put("id", id);
    out.put("object", object);
    out.put("created", created);
    out.put("owned_by", ownedBy);
    out.put("ability_ref", abilityRef);
    out.put("metadata", metadata);
    return out;
  }
}
