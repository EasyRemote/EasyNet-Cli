package run.easynet.daemon;

import java.util.Map;

public record MissionChildReceipt(
    String stepID, String invocationURA, String receiptURA, String receiptHash) {
  public MissionChildReceipt {
    stepID = stepID == null ? "" : stepID;
    invocationURA = invocationURA == null ? "" : invocationURA;
    receiptURA = receiptURA == null ? "" : receiptURA;
    receiptHash = receiptHash == null ? "" : receiptHash;
    MissionSupport.requireChildReceiptFact(receiptURA, receiptHash);
  }

  static MissionChildReceipt fromObject(Map<String, Object> fields) {
    return new MissionChildReceipt(
        MissionSupport.optionalString(fields, "step_id"),
        MissionSupport.optionalString(fields, "invocation_ura"),
        MissionSupport.requiredString(fields, "receipt_ura"),
        MissionSupport.requiredString(fields, "receipt_hash"));
  }
}
