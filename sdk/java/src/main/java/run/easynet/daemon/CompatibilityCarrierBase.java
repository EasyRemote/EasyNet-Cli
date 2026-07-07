package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record CompatibilityCarrierBase(
    String callerURA,
    String calleeURA,
    String subjectURA,
    String descriptorVersion,
    String nonceBase64,
    Map<String, Object> causalContext,
    String authToken,
    Map<String, Object> metadata) {
  public CompatibilityCarrierBase {
    callerURA = CompatibilitySupport.requiredURA(callerURA, "caller_ura");
    calleeURA = CompatibilitySupport.requiredURA(calleeURA, "callee_ura");
    subjectURA = CompatibilitySupport.requiredURA(subjectURA, "subject_ura");
    descriptorVersion = CompatibilitySupport.requiredString(descriptorVersion, "descriptor_version");
    nonceBase64 = CompatibilitySupport.requiredString(nonceBase64, "nonce_base64");
    causalContext = CompatibilitySupport.copyObject(causalContext);
    if (causalContext.isEmpty()) {
      throw CompatibilitySupport.invalid("causal_context is required");
    }
    authToken = CompatibilitySupport.optionalString(authToken, "auth_token");
    metadata = CompatibilitySupport.copyObject(metadata);
  }

  static CompatibilityCarrierBase fromObject(Map<String, Object> fields) {
    return new CompatibilityCarrierBase(
        CompatibilitySupport.requiredString(fields.get("caller_ura"), "caller_ura"),
        CompatibilitySupport.requiredString(fields.get("callee_ura"), "callee_ura"),
        CompatibilitySupport.requiredString(fields.get("subject_ura"), "subject_ura"),
        CompatibilitySupport.requiredString(fields.get("descriptor_version"), "descriptor_version"),
        CompatibilitySupport.requiredString(fields.get("nonce_base64"), "nonce_base64"),
        CompatibilitySupport.requiredObject(fields.get("causal_context"), "causal_context"),
        CompatibilitySupport.optionalString(fields.get("auth_token"), "auth_token"),
        fields.containsKey("metadata") ? CompatibilitySupport.requiredObject(fields.get("metadata"), "metadata") : Map.of());
  }

  LinkedHashMap<String, Object> toObject() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("caller_ura", callerURA);
    out.put("callee_ura", calleeURA);
    out.put("subject_ura", subjectURA);
    out.put("descriptor_version", descriptorVersion);
    out.put("nonce_base64", nonceBase64);
    out.put("causal_context", causalContext);
    CompatibilitySupport.putOptional(out, "auth_token", authToken);
    if (!metadata.isEmpty()) {
      out.put("metadata", metadata);
    }
    return out;
  }
}
