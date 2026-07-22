package run.runtime.sdk;

import java.util.LinkedHashMap;
import java.util.Map;

public record AuthorityMetadata(String kind, String key, String value) {
  public AuthorityMetadata {
    kind = AuthoritySupport.requiredString(kind, "kind");
    key = AuthoritySupport.requiredString(key, "key");
    value = AuthoritySupport.requiredString(value, "value");
    if (!AuthoritySupport.DELEGATION_KIND.equals(kind)
        && !AuthoritySupport.SESSION_AUTHORITY_KIND.equals(kind)) {
      throw AuthoritySupport.invalid("authority kind is not supported");
    }
    if (!AuthoritySupport.DELEGATION_METADATA_KEY.equals(key)
        && !AuthoritySupport.SESSION_AUTHORITY_METADATA_KEY.equals(key)) {
      throw AuthoritySupport.invalid("authority metadata key is not supported");
    }
    if ((AuthoritySupport.DELEGATION_KIND.equals(kind)
            && !AuthoritySupport.DELEGATION_METADATA_KEY.equals(key))
        || (AuthoritySupport.SESSION_AUTHORITY_KIND.equals(kind)
            && !AuthoritySupport.SESSION_AUTHORITY_METADATA_KEY.equals(key))) {
      throw AuthoritySupport.invalid("authority kind and metadata key mismatch");
    }
  }

  public Map<String, Object> toMetadata() {
    return Map.of(key, value);
  }

  public Map<String, Object> mergeInto(Map<String, Object> metadata) {
    LinkedHashMap<String, Object> merged = new LinkedHashMap<>();
    if (metadata != null) {
      merged.putAll(metadata);
    }
    merged.put(key, value);
    AuthoritySupport.validateAuthorityMetadata(merged);
    return Map.copyOf(merged);
  }
}
