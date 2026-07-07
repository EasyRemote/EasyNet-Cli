package run.easynet.daemon;

import java.util.Map;

public record MissionChildInvocation(
    String stepID,
    String requestID,
    String traceID,
    String ability,
    String invocationURA,
    String callerURA,
    String calleeURA,
    String subjectURA,
    String metadataState,
    Object ledgerState,
    Map<String, Object> receipt) {
  public MissionChildInvocation {
    receipt = MissionSupport.copyObject(receipt);
    MissionSupport.requireChildInvocationFact(
        stepID, requestID, traceID, ability, invocationURA, callerURA, calleeURA, subjectURA, receipt);
  }

  static MissionChildInvocation fromObject(Map<String, Object> fields) {
    return new MissionChildInvocation(
        MissionSupport.optionalString(fields, "step_id"),
        MissionSupport.optionalString(fields, "request_id"),
        MissionSupport.optionalString(fields, "trace_id"),
        MissionSupport.optionalString(fields, "ability"),
        MissionSupport.optionalString(fields, "invocation_ura"),
        MissionSupport.optionalString(fields, "caller_ura"),
        MissionSupport.optionalString(fields, "callee_ura"),
        MissionSupport.optionalString(fields, "subject_ura"),
        MissionSupport.optionalString(fields, "metadata_state"),
        fields.get("ledger_state"),
        MissionSupport.optionalObject(fields.get("receipt"), "receipt"));
  }
}
