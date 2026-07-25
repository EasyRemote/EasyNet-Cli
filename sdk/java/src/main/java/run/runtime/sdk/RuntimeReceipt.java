package run.runtime.sdk;

import java.util.Map;

public final class RuntimeReceipt {
  private final Map<String, Object> raw;
  private final String invocationId;
  private final String receiptType;
  private final String state;

  private RuntimeReceipt(Map<String, Object> raw) {
    this.raw = RuntimeReceiptProofFacts.immutableObject(raw, "runtime receipt");
    this.invocationId = requiredString(this.raw, "invocation_id");
    this.receiptType = requiredString(this.raw, "receipt_type");
    this.state = requiredString(this.raw, "state");
    validateSummary();
  }

  public static RuntimeReceipt fromMap(Map<String, Object> raw) {
    return new RuntimeReceipt(raw);
  }

  public Map<String, Object> raw() {
    return raw;
  }

  public String invocationId() {
    return invocationId;
  }

  public String receiptType() {
    return receiptType;
  }

  public String state() {
    return state;
  }

  public String lifecycleState() {
    return canonicalLifecycleState(state);
  }

  public Map<String, Object> rawProjection() {
    return raw;
  }

  private void validateSummary() {
    if (invocationId.isBlank()) {
      throw SDKError.validation(
          "runtime_receipt", "runtime receipt summary is missing invocation_id");
    }
    if (receiptType.isBlank()) {
      throw SDKError.validation(
          "runtime_receipt", "runtime receipt summary is missing receipt_type");
    }
    String lifecycleState = canonicalLifecycleState(state);
    if (lifecycleState.equals("UNSPECIFIED")) {
      throw SDKError.validation(
          "runtime_receipt", "runtime receipt lifecycle state must not be UNSPECIFIED");
    }
    if (!receiptType.equals(canonicalReceiptType(lifecycleState))) {
      throw SDKError.validation(
          "runtime_receipt", "runtime receipt receipt_type does not match its lifecycle state");
    }
    RuntimeReceiptProofFacts.receiptHash(raw, "prev_receipt_hash_hex", true);
    RuntimeReceiptProofFacts.receiptHash(raw, "self_hash_hex", false);
    RuntimeReceiptProofFacts.validate(raw);
  }

  private static String canonicalLifecycleState(String value) {
    return switch (value.trim()) {
      case "accepted", "Accepted", "ACCEPTED" -> "ACCEPTED";
      case "admitted", "Admitted", "ADMITTED" -> "ADMITTED";
      case "dispatched", "Dispatched", "DISPATCHED" -> "DISPATCHED";
      case "running", "Running", "RUNNING" -> "RUNNING";
      case "completed", "Completed", "COMPLETED" -> "COMPLETED";
      case "failed", "Failed", "FAILED" -> "FAILED";
      case "timed_out", "TimedOut", "TIMED_OUT" -> "TIMED_OUT";
      case "cancelled", "Cancelled", "CANCELLED" -> "CANCELLED";
      case "unspecified", "Unspecified", "UNSPECIFIED" -> "UNSPECIFIED";
      default -> throw SDKError.validation("runtime_receipt", "unknown receipt state " + value);
    };
  }

  private static String canonicalReceiptType(String lifecycleState) {
    return switch (lifecycleState) {
      case "ACCEPTED" -> "accepted";
      case "ADMITTED" -> "admitted";
      case "DISPATCHED" -> "dispatched";
      case "RUNNING" -> "running";
      case "COMPLETED" -> "completed";
      case "FAILED" -> "failed";
      case "TIMED_OUT" -> "timed_out";
      case "CANCELLED" -> "cancelled";
      default -> "";
    };
  }

  private static String requiredString(Map<String, Object> raw, String field) {
    Object value = raw.get(field);
    if (!(value instanceof String string) || string.isBlank()) {
      throw SDKError.validation("runtime_receipt", "runtime receipt summary is missing " + field);
    }
    return string;
  }
}
