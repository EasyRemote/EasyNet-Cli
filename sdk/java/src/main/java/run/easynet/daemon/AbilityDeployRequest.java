package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record AbilityDeployRequest(
    String callerURA,
    String calleeURA,
    String subjectURA,
    String descriptorVersion,
    String nonceBase64,
    Map<String, Object> causalContext,
    ResourceRef resourceRef,
    String nodeID,
    Map<String, Object> metadata) {
  public AbilityDeployRequest {
    callerURA = PublicationSupport.required(callerURA, "caller_ura");
    calleeURA = PublicationSupport.required(calleeURA, "callee_ura");
    subjectURA = PublicationSupport.required(subjectURA, "subject_ura");
    descriptorVersion = PublicationSupport.required(descriptorVersion, "descriptor_version");
    nonceBase64 = PublicationSupport.required(nonceBase64, "nonce_base64");
    causalContext = PublicationSupport.copyObject(causalContext);
    if (causalContext.isEmpty()) {
      throw PublicationSupport.invalid("causal_context is required");
    }
    if (resourceRef == null) {
      throw PublicationSupport.invalid("resource_ref is required");
    }
    nodeID = PublicationSupport.required(nodeID, "node_id");
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
    out.put("resource_ref", resourceRef.toObject());
    out.put("node_id", nodeID);
    if (!metadata.isEmpty()) {
      out.put("metadata", metadata);
    }
    return JsonValueWriter.object(out);
  }
}
