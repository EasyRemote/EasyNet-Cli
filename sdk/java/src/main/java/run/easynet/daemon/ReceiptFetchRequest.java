package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record ReceiptFetchRequest(
    String callerURA,
    String calleeURA,
    String descriptorRef,
    String subjectURA,
    String descriptorVersion,
    String nonceBase64,
    Map<String, Object> causalContext,
    String invocationURA,
    String requestID,
    String traceID,
    Map<String, Object> metadata) {
  public ReceiptFetchRequest {
    callerURA = ReceiptSupport.required(callerURA, "caller_ura");
    calleeURA = ReceiptSupport.required(calleeURA, "callee_ura");
    descriptorRef = ReceiptSupport.required(descriptorRef, "descriptor_ref");
    subjectURA = ReceiptSupport.required(subjectURA, "subject_ura");
    descriptorVersion = ReceiptSupport.required(descriptorVersion, "descriptor_version");
    nonceBase64 = ReceiptSupport.required(nonceBase64, "nonce_base64");
    causalContext = ReceiptSupport.requiredObject(causalContext, "causal_context");
    invocationURA = ReceiptSupport.optional(invocationURA, "invocation_ura");
    requestID = ReceiptSupport.optional(requestID, "request_id");
    traceID = ReceiptSupport.optional(traceID, "trace_id");
    metadata = metadata == null ? Map.of() : Map.copyOf(metadata);
    int selectors = (invocationURA.isEmpty() ? 0 : 1) + (requestID.isEmpty() ? 0 : 1) + (traceID.isEmpty() ? 0 : 1);
    if (selectors != 1) {
      throw ReceiptSupport.invalid("exactly one receipt fetch selector is required");
    }
  }

  byte[] toJSON() {
    return JsonValueWriter.object(toMap());
  }

  Map<String, Object> toMap() {
    LinkedHashMap<String, Object> value = new LinkedHashMap<>();
    value.put("caller_ura", callerURA);
    value.put("callee_ura", calleeURA);
    value.put("descriptor_ref", descriptorRef);
    value.put("subject_ura", subjectURA);
    value.put("descriptor_version", descriptorVersion);
    value.put("nonce_base64", nonceBase64);
    value.put("causal_context", causalContext);
    if (!invocationURA.isEmpty()) {
      value.put("invocation_ura", invocationURA);
    }
    if (!requestID.isEmpty()) {
      value.put("request_id", requestID);
    }
    if (!traceID.isEmpty()) {
      value.put("trace_id", traceID);
    }
    if (!metadata.isEmpty()) {
      value.put("metadata", metadata);
    }
    return value;
  }
}
