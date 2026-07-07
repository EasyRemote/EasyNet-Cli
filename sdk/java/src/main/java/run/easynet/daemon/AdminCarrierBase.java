package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record AdminCarrierBase(
    String callerURA,
    String calleeURA,
    String subjectURA,
    String descriptorVersion,
    String nonceBase64,
    Map<String, Object> causalContext,
    Map<String, Object> metadata) {
  public AdminCarrierBase {
    callerURA = AdminSupport.required(callerURA, "caller_ura");
    calleeURA = AdminSupport.required(calleeURA, "callee_ura");
    subjectURA = AdminSupport.required(subjectURA, "subject_ura");
    descriptorVersion = AdminSupport.required(descriptorVersion, "descriptor_version");
    nonceBase64 = AdminSupport.required(nonceBase64, "nonce_base64");
    causalContext = AdminSupport.copyObject(causalContext);
    if (causalContext.isEmpty()) {
      throw AdminSupport.invalid("causal_context is required");
    }
    metadata = AdminSupport.copyObject(metadata);
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
