package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record DirectoryResolveRequest(
    String callerURA,
    String calleeURA,
    String subjectURA,
    String descriptorVersion,
    String nonceBase64,
    Map<String, Object> causalContext,
    String queryName,
    String abilityName,
    String qtype,
    String realmHint,
    Map<String, Object> metadata) {
  public DirectoryResolveRequest {
    callerURA = DirectoryIdentitySupport.cleanRequired(callerURA, "caller_ura");
    calleeURA = DirectoryIdentitySupport.cleanRequired(calleeURA, "callee_ura");
    subjectURA = DirectoryIdentitySupport.cleanRequired(subjectURA, "subject_ura");
    descriptorVersion =
        DirectoryIdentitySupport.cleanRequired(descriptorVersion, "descriptor_version");
    nonceBase64 = DirectoryIdentitySupport.cleanRequired(nonceBase64, "nonce_base64");
    causalContext = DirectoryIdentitySupport.requiredCopiedObject(causalContext, "causal_context");
    queryName = DirectoryIdentitySupport.optionalClean(queryName, "query_name");
    abilityName = DirectoryIdentitySupport.optionalClean(abilityName, "ability_name");
    qtype = DirectoryIdentitySupport.optionalClean(qtype, "qtype");
    realmHint = DirectoryIdentitySupport.optionalClean(realmHint, "realm_hint");
    metadata = DirectoryIdentitySupport.copyObject(metadata);
    if (queryName == null && realmHint == null) {
      throw SDKError.validation("directory", "query_name or realm_hint is required");
    }
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("caller_ura", callerURA);
    out.put("callee_ura", calleeURA);
    out.put("subject_ura", subjectURA);
    out.put("descriptor_version", descriptorVersion);
    out.put("nonce_base64", nonceBase64);
    out.put("causal_context", causalContext);
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
