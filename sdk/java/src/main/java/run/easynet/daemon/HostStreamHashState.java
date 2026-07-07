package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record HostStreamHashState(
    String algorithm,
    String outputHash,
    long frames,
    Long lastSeq,
    String canonicalJSON) {
  public HostStreamHashState {
    HostBindingSupport.validateHashState(algorithm, outputHash, frames, lastSeq);
    canonicalJSON = canonicalJSON == null ? "" : canonicalJSON;
  }

  public static HostStreamHashState initial() {
    return new HostStreamHashState(
        HostBindingSupport.HASH_ALGORITHM, HostBindingSupport.EMPTY_OUTPUT_HASH, 0, null, "");
  }

  public static HostStreamHashState fromJSON(byte[] raw) {
    var fields = JsonValueReader.object(raw, "host stream hash state JSON");
    return fromObject(fields);
  }

  static HostStreamHashState fromObject(Map<String, Object> fields) {
    return new HostStreamHashState(
        HostBindingSupport.requiredString(fields, "algorithm"),
        HostBindingSupport.requiredString(fields, "output_hash"),
        HostBindingSupport.requiredLong(fields, "frames"),
        HostBindingSupport.optionalLong(fields.get("last_seq"), "last_seq"),
        HostBindingSupport.optionalString(fields.get("canonical_json"), "canonical_json"));
  }

  Map<String, Object> toObject() {
    LinkedHashMap<String, Object> object = new LinkedHashMap<>();
    object.put("algorithm", algorithm);
    object.put("output_hash", outputHash);
    object.put("frames", frames);
    object.put("last_seq", lastSeq);
    if (!canonicalJSON.isEmpty()) {
      object.put("canonical_json", canonicalJSON);
    }
    return object;
  }

  public HostStreamHashState fold(long seq, Object value) {
    return HostBindingSupport.foldOutputHash(this, seq, value);
  }
}
