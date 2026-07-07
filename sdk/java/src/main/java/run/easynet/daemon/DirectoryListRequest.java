package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record DirectoryListRequest(
    String callerURA,
    String calleeURA,
    String subjectURA,
    String descriptorVersion,
    String nonceBase64,
    Map<String, Object> causalContext,
    Integer limit,
    String cursor,
    String scope,
    String ownerURA,
    String abilityURA,
    Map<String, Object> metadata) {
  public DirectoryListRequest {
    callerURA = DirectoryIdentitySupport.cleanRequired(callerURA, "caller_ura");
    calleeURA = DirectoryIdentitySupport.cleanRequired(calleeURA, "callee_ura");
    subjectURA = DirectoryIdentitySupport.cleanRequired(subjectURA, "subject_ura");
    descriptorVersion =
        DirectoryIdentitySupport.cleanRequired(descriptorVersion, "descriptor_version");
    nonceBase64 = DirectoryIdentitySupport.cleanRequired(nonceBase64, "nonce_base64");
    causalContext = DirectoryIdentitySupport.requiredCopiedObject(causalContext, "causal_context");
    limit = DirectoryIdentitySupport.normalizeLimit(limit);
    cursor = DirectoryIdentitySupport.optionalClean(cursor, "cursor");
    scope = DirectoryIdentitySupport.optionalClean(scope, "scope");
    ownerURA = DirectoryIdentitySupport.optionalClean(ownerURA, "owner_ura");
    abilityURA = DirectoryIdentitySupport.optionalClean(abilityURA, "ability_ura");
    metadata = DirectoryIdentitySupport.copyObject(metadata);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = baseJSON();
    if (scope != null) {
      out.put("scope", scope);
    }
    if (ownerURA != null) {
      out.put("owner_ura", ownerURA);
    }
    if (abilityURA != null) {
      out.put("ability_ura", abilityURA);
    }
    return JsonValueWriter.object(out);
  }

  private LinkedHashMap<String, Object> baseJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("caller_ura", callerURA);
    out.put("callee_ura", calleeURA);
    out.put("subject_ura", subjectURA);
    out.put("descriptor_version", descriptorVersion);
    out.put("nonce_base64", nonceBase64);
    out.put("causal_context", causalContext);
    out.put("limit", limit);
    if (cursor != null) {
      out.put("cursor", cursor);
    }
    if (!metadata.isEmpty()) {
      out.put("metadata", metadata);
    }
    return out;
  }
}
