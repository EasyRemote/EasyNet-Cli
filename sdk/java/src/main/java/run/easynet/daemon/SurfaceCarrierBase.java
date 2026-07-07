package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record SurfaceCarrierBase(
    String callerURA,
    String calleeURA,
    String subjectURA,
    String descriptorVersion,
    String nonceBase64,
    Map<String, Object> causalContext,
    Map<String, Object> metadata) {
  public SurfaceCarrierBase {
    callerURA = SurfaceSupport.cleanRequired(callerURA, "caller_ura");
    calleeURA = SurfaceSupport.cleanRequired(calleeURA, "callee_ura");
    subjectURA = SurfaceSupport.cleanRequired(subjectURA, "subject_ura");
    descriptorVersion = SurfaceSupport.cleanRequired(descriptorVersion, "descriptor_version");
    nonceBase64 = SurfaceSupport.cleanRequired(nonceBase64, "nonce_base64");
    causalContext = SurfaceSupport.copyObject(causalContext);
    if (causalContext.isEmpty()) {
      throw SurfaceSupport.invalid("causal_context is required");
    }
    metadata = SurfaceSupport.copyObject(metadata);
  }

  LinkedHashMap<String, Object> toObject() {
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
