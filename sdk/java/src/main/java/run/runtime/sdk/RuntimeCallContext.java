package run.runtime.sdk;

import java.util.LinkedHashMap;
import java.util.Map;

public record RuntimeCallContext(
    String callerURA,
    String calleeURA,
    String subjectURA,
    String nonceBase64,
    Map<String, Object> causalContext,
    Map<String, Object> metadata) {
  public RuntimeCallContext(
      String callerURA,
      String calleeURA,
      String subjectURA,
      String nonceBase64,
      Map<String, Object> causalContext) {
    this(callerURA, calleeURA, subjectURA, nonceBase64, causalContext, Map.of());
  }

  public RuntimeCallContext {
    callerURA = requiredPrincipal(callerURA, "caller_ura");
    calleeURA = requiredPrincipal(calleeURA, "callee_ura");
    subjectURA = requiredPrincipal(subjectURA, "subject_ura");
    nonceBase64 = required(nonceBase64, "nonce_base64");
    causalContext = copyObject(causalContext, "causal_context");
    metadata = copyObject(metadata, "metadata");
  }

  private static Map<String, Object> copyObject(Map<String, Object> value, String field) {
    if (value == null || value.isEmpty()) {
      return Map.of();
    }
    Map<String, Object> out = new LinkedHashMap<>();
    for (Map.Entry<String, Object> entry : value.entrySet()) {
      if (entry.getKey() == null || entry.getKey().isBlank()) {
        throw SDKError.validation("runtime", field + " keys must be non-empty strings");
      }
      out.put(entry.getKey(), entry.getValue());
    }
    return Map.copyOf(out);
  }

  private static String required(String value, String field) {
    String clean = value == null ? "" : value.trim();
    if (clean.isBlank()) {
      throw SDKError.validation("runtime", field + " is required");
    }
    return clean;
  }

  private static String requiredPrincipal(String value, String field) {
    String clean = required(value, field);
    if (RuntimePrincipals.containsAllZeroPrincipal(clean)) {
      throw SDKError.validation("runtime", field + " must not be all-zero");
    }
    return clean;
  }
}
