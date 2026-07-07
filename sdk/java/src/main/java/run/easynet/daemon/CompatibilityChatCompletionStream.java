package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public record CompatibilityChatCompletionStream(
    String profile,
    String kind,
    boolean stream,
    List<CompatibilityChatCompletionChunk> items,
    String doneSentinel,
    Map<String, Object> metadata) {
  public CompatibilityChatCompletionStream {
    CompatibilitySupport.validateKind(profile, kind, "chat_completion_stream");
    if (!stream || !"[DONE]".equals(doneSentinel)) {
      throw CompatibilitySupport.invalid("invalid chat_completion_stream projection");
    }
    items = List.copyOf(items);
    metadata = CompatibilitySupport.copyObject(metadata);
  }

  public static CompatibilityChatCompletionStream fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "compatibility chat stream JSON");
    Object streamValue = fields.get("stream");
    if (!(streamValue instanceof Boolean bool)) {
      throw CompatibilitySupport.invalid("stream must be a boolean");
    }
    List<CompatibilityChatCompletionChunk> chunks = CompatibilitySupport.requiredList(fields.get("items"), "items").stream()
        .map(item -> CompatibilityChatCompletionChunk.fromObject(CompatibilitySupport.requiredObject(item, "chunk")))
        .toList();
    return new CompatibilityChatCompletionStream(
        CompatibilitySupport.requiredString(fields.get("profile"), "profile"),
        CompatibilitySupport.requiredString(fields.get("kind"), "kind"),
        bool,
        chunks,
        CompatibilitySupport.requiredString(fields.get("done_sentinel"), "done_sentinel"),
        CompatibilitySupport.requiredObject(fields.get("metadata"), "metadata"));
  }

  public byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("profile", profile);
    out.put("kind", kind);
    out.put("stream", stream);
    out.put("items", items.stream().map(CompatibilityChatCompletionChunk::toObject).toList());
    out.put("done_sentinel", doneSentinel);
    out.put("metadata", metadata);
    return JsonValueWriter.object(out);
  }
}
