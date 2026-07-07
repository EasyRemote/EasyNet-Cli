package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record MissionCarrierBase(
    String callerURA,
    String calleeURA,
    String subjectURA,
    String descriptorVersion,
    String nonceBase64,
    Map<String, Object> causalContext,
    Map<String, Object> metadata) {
  public MissionCarrierBase {
    callerURA = MissionSupport.required(callerURA, "caller_ura");
    calleeURA = MissionSupport.required(calleeURA, "callee_ura");
    subjectURA = MissionSupport.required(subjectURA, "subject_ura");
    descriptorVersion = MissionSupport.required(descriptorVersion, "descriptor_version");
    nonceBase64 = MissionSupport.required(nonceBase64, "nonce_base64");
    causalContext = MissionSupport.copyObject(causalContext);
    if (causalContext.isEmpty()) {
      throw MissionSupport.invalid("causal_context is required");
    }
    metadata = MissionSupport.copyObject(metadata);
  }

  Map<String, Object> toObject() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("caller_ura", callerURA);
    out.put("callee_ura", calleeURA);
    out.put("subject_ura", subjectURA);
    out.put("descriptor_version", descriptorVersion);
    out.put("nonce_base64", nonceBase64);
    out.put("causal_context", causalContext);
    if (!metadata.isEmpty()) {
      out.put("metadata", metadata);
    }
    return out;
  }
}
