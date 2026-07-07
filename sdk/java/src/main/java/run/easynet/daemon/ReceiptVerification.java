package run.easynet.daemon;

import java.util.Map;

public record ReceiptVerification(
    boolean verified,
    String method,
    String receiptURA,
    String invocationID,
    String reason,
    Map<String, Object> metadata) {
  public ReceiptVerification {
    method = ReceiptSupport.required(method, "method");
    receiptURA = receiptURA == null ? "" : receiptURA;
    invocationID = invocationID == null ? "" : invocationID;
    reason = reason == null ? "" : reason;
    metadata = metadata == null ? Map.of() : Map.copyOf(metadata);
    if (verified && method.equals("summary-only")) {
      throw ReceiptSupport.invalid("summary-only projection cannot be verified");
    }
  }

  public boolean isCryptographic() {
    if (!verified) {
      return false;
    }
    String normalized = method.trim().toLowerCase().replace('_', '-');
    Object assurance = metadata.get("assurance");
    return normalized.startsWith("axon-")
        || normalized.equals("full-receipt")
        || normalized.equals("full-receipt-verification")
        || normalized.equals("cryptographic")
        || "cryptographic".equals(assurance)
        || "axon-cryptographic".equals(assurance);
  }

  public ReceiptVerification requireCryptographic() {
    if (!isCryptographic()) {
      throw ReceiptSupport.invalid("receipt verification is not Axon-backed cryptographic evidence");
    }
    return this;
  }
}
