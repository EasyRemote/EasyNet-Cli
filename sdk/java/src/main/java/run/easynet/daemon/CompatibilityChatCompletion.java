package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public record CompatibilityChatCompletion(
    String profile,
    String kind,
    String id,
    String object,
    long created,
    String model,
    List<Object> choices,
    Map<String, Object> usage,
    Map<String, Object> metadata) {
  public CompatibilityChatCompletion {
    CompatibilitySupport.validateKind(profile, kind, "chat_completion");
    CompatibilitySupport.validateObject(object, "chat.completion", "chat_completion");
    id = CompatibilitySupport.requiredString(id, "id");
    if (created < 0) {
      throw CompatibilitySupport.invalid("created must be a non-negative integer");
    }
    model = CompatibilitySupport.requiredAbilityURA(model, "model");
    choices = List.copyOf(choices);
    usage = CompatibilitySupport.copyObject(usage);
    metadata = CompatibilitySupport.copyObject(metadata);
  }

  public static CompatibilityChatCompletion fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "compatibility chat completion JSON");
    return new CompatibilityChatCompletion(
        CompatibilitySupport.requiredString(fields.get("profile"), "profile"),
        CompatibilitySupport.requiredString(fields.get("kind"), "kind"),
        CompatibilitySupport.requiredString(fields.get("id"), "id"),
        CompatibilitySupport.requiredString(fields.get("object"), "object"),
        CompatibilitySupport.requiredNonNegativeInteger(fields.get("created"), "created"),
        CompatibilitySupport.requiredString(fields.get("model"), "model"),
        CompatibilitySupport.requiredList(fields.get("choices"), "choices"),
        CompatibilitySupport.requiredObject(fields.get("usage"), "usage"),
        CompatibilitySupport.requiredObject(fields.get("metadata"), "metadata"));
  }

  public byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("profile", profile);
    out.put("kind", kind);
    out.put("id", id);
    out.put("object", object);
    out.put("created", created);
    out.put("model", model);
    out.put("choices", choices);
    out.put("usage", usage);
    out.put("metadata", metadata);
    return JsonValueWriter.object(out);
  }
}
