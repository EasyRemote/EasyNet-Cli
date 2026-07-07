package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record EventsCarrierBase(
    String callerURA,
    String calleeURA,
    String subjectURA,
    String descriptorVersion,
    String nonceBase64,
    Map<String, Object> causalContext,
    Map<String, Object> metadata) {
  public EventsCarrierBase {
    callerURA = EventsSupport.cleanRequired(callerURA, "caller_ura");
    calleeURA = EventsSupport.cleanRequired(calleeURA, "callee_ura");
    subjectURA = EventsSupport.cleanRequired(subjectURA, "subject_ura");
    descriptorVersion = EventsSupport.cleanRequired(descriptorVersion, "descriptor_version");
    nonceBase64 = EventsSupport.cleanRequired(nonceBase64, "nonce_base64");
    causalContext = EventsSupport.requiredCopiedObject(causalContext, "causal_context");
    metadata = EventsSupport.copyObject(metadata);
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
