package run.easynet.daemon;

import java.util.Map;

public record ResolveQuery(
    DirectoryQueryBase base,
    String queryName,
    String abilityName,
    String qtype,
    String realmHint,
    Map<String, Object> metadata) {
  public ResolveQuery {
    if (base == null) {
      throw SDKError.validation("directory", "base query is required");
    }
    queryName = DirectoryIdentitySupport.optionalClean(queryName, "query_name");
    abilityName = DirectoryIdentitySupport.optionalClean(abilityName, "ability_name");
    qtype = DirectoryIdentitySupport.optionalClean(qtype, "qtype");
    realmHint = DirectoryIdentitySupport.optionalClean(realmHint, "realm_hint");
    metadata = metadata == null ? base.metadata() : DirectoryIdentitySupport.copyObject(metadata);
    if (queryName == null && realmHint == null) {
      throw SDKError.validation("directory", "query_name or realm_hint is required");
    }
  }

  byte[] toJSON() {
    Map<String, Object> out = base.toObject();
    if (queryName != null) {
      out.put("query_name", queryName);
    }
    if (abilityName != null) {
      out.put("ability_name", abilityName);
    }
    if (qtype != null) {
      out.put("qtype", qtype);
    }
    if (realmHint != null) {
      out.put("realm_hint", realmHint);
    }
    if (!metadata.isEmpty()) {
      out.put("metadata", metadata);
    }
    return JsonValueWriter.object(out);
  }
}
