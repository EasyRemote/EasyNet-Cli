package run.runtime.sdk;

import java.util.Map;

public final class RuntimeReceipt {
  private final Map<String, Object> raw;
  private final String invocationId;
  private final String receiptType;
  private final String state;

  private RuntimeReceipt(Map<String, Object> raw) {
    this.raw = RuntimeReceiptProofFacts.immutableObject(raw, "runtime receipt");
    RuntimeReceiptProofFacts.validate(this.raw);
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
  }

  private static String canonicalLifecycleState(String value) {
    return switch (value) {
      case "Accepted" -> "ACCEPTED";
      case "Admitted" -> "ADMITTED";
      case "Dispatched" -> "DISPATCHED";
      case "Running" -> "RUNNING";
      case "Completed" -> "COMPLETED";
      case "Failed" -> "FAILED";
      case "TimedOut" -> "TIMED_OUT";
      case "Cancelled" -> "CANCELLED";
      case "Unspecified" -> "UNSPECIFIED";
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
      default ->
          throw SDKError.validation(
              "runtime_receipt", "unknown canonical receipt lifecycle state " + lifecycleState);
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
