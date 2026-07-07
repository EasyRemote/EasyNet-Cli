package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record HostStreamTerminalSummary(
    String outputHash,
    long frames,
    Map<String, Object> metadata) {
  public HostStreamTerminalSummary {
    if (!HostBindingSupport.isOutputHash(outputHash)) {
      throw HostBindingSupport.invalid("terminal output_hash must be a sha256 digest");
    }
    if (frames < 0) {
      throw HostBindingSupport.invalid("terminal frames must be non-negative");
    }
    metadata = HostBindingSupport.copyObject(metadata);
  }

  public static HostStreamTerminalSummary fromJSON(byte[] raw) {
    return fromObject(JsonValueReader.object(raw, "host stream terminal summary JSON"));
  }

  static HostStreamTerminalSummary fromObject(Map<String, Object> fields) {
    return new HostStreamTerminalSummary(
        HostBindingSupport.requiredString(fields, "output_hash"),
        HostBindingSupport.requiredLong(fields, "frames"),
        HostBindingSupport.optionalObject(fields.get("metadata"), "metadata"));
  }

  Map<String, Object> toObject() {
    LinkedHashMap<String, Object> object = new LinkedHashMap<>();
    object.put("output_hash", outputHash);
    object.put("frames", frames);
    if (!metadata.isEmpty()) {
      object.put("metadata", metadata);
    }
    return object;
  }
}
