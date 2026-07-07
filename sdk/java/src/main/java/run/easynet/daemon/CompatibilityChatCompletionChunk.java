package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public record CompatibilityChatCompletionChunk(
    String profile,
    String kind,
    String id,
    String object,
    long created,
    String model,
    List<Object> choices,
    Map<String, Object> usage,
    Map<String, Object> metadata) {
  public CompatibilityChatCompletionChunk {
    CompatibilitySupport.validateKind(profile, kind, "chat_completion_chunk");
    CompatibilitySupport.validateObject(object, "chat.completion.chunk", "chat_completion_chunk");
    id = CompatibilitySupport.requiredString(id, "id");
    if (created < 0) {
      throw CompatibilitySupport.invalid("created must be a non-negative integer");
    }
    model = CompatibilitySupport.requiredAbilityURA(model, "model");
    choices = List.copyOf(choices);
    usage = usage == null ? null : CompatibilitySupport.copyObject(usage);
    metadata = CompatibilitySupport.copyObject(metadata);
  }

  static CompatibilityChatCompletionChunk fromObject(Map<String, Object> fields) {
    return new CompatibilityChatCompletionChunk(
        CompatibilitySupport.requiredString(fields.get("profile"), "profile"),
        CompatibilitySupport.requiredString(fields.get("kind"), "kind"),
        CompatibilitySupport.requiredString(fields.get("id"), "id"),
        CompatibilitySupport.requiredString(fields.get("object"), "object"),
        CompatibilitySupport.requiredNonNegativeInteger(fields.get("created"), "created"),
        CompatibilitySupport.requiredString(fields.get("model"), "model"),
        CompatibilitySupport.requiredList(fields.get("choices"), "choices"),
        CompatibilitySupport.optionalObject(fields.get("usage"), "usage"),
        CompatibilitySupport.requiredObject(fields.get("metadata"), "metadata"));
  }

  Map<String, Object> toObject() {
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
    return out;
  }
}
