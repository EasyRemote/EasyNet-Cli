package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record UnpublishAbilityRequest(
    String callerURA,
    String calleeURA,
    String subjectURA,
    String descriptorVersion,
    String nonceBase64,
    Map<String, Object> causalContext,
    String abilityURA,
    Map<String, Object> metadata) {
  public UnpublishAbilityRequest {
    callerURA = PublicationSupport.required(callerURA, "caller_ura");
    calleeURA = PublicationSupport.required(calleeURA, "callee_ura");
    subjectURA = PublicationSupport.required(subjectURA, "subject_ura");
    descriptorVersion = PublicationSupport.required(descriptorVersion, "descriptor_version");
    nonceBase64 = PublicationSupport.required(nonceBase64, "nonce_base64");
    causalContext = PublicationSupport.copyObject(causalContext);
    if (causalContext.isEmpty()) {
      throw PublicationSupport.invalid("causal_context is required");
    }
    abilityURA = PublicationSupport.required(abilityURA, "ability_ura");
    metadata = PublicationSupport.copyObject(metadata);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("caller_ura", callerURA);
    out.put("callee_ura", calleeURA);
    out.put("subject_ura", subjectURA);
    out.put("descriptor_version", descriptorVersion);
    out.put("nonce_base64", nonceBase64);
    out.put("causal_context", causalContext);
    out.put("ability_ura", abilityURA);
    if (!metadata.isEmpty()) {
      out.put("metadata", metadata);
    }
    return JsonValueWriter.object(out);
  }
}
