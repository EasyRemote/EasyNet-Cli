package run.easynet.daemon;

import java.util.Map;
import java.util.Objects;

public record ReceiptSummary(
    String receiptURA,
    String invocationID,
    String state,
    boolean verified,
    Object output,
    Object error,
    String causalRef,
    Map<String, Object> metadata) {
  public ReceiptSummary {
    state = ReceiptSupport.required(state, "state");
    metadata = metadata == null ? Map.of() : Map.copyOf(metadata);
  }

  public static ReceiptSummary fromJSON(byte[] raw) {
    Objects.requireNonNull(raw, "raw");
    Map<String, Object> fields = JsonValueReader.object(raw, "receipt summary JSON");
    if (!fields.containsKey("output")) {
      throw ReceiptSupport.invalid("output is required");
    }
    return new ReceiptSummary(
        ReceiptSupport.optionalJSON(fields, "receipt_ura"),
        ReceiptSupport.optionalJSON(fields, "invocation_id"),
        ReceiptSupport.requiredJSON(fields, "state"),
        ReceiptSupport.requiredBoolean(fields, "verified"),
        fields.get("output"),
        fields.get("error"),
        ReceiptSupport.optionalJSON(fields, "causal_ref"),
        ReceiptSupport.optionalObject(fields, "metadata"));
  }

  public ReceiptVerification summaryVerification() {
    return new ReceiptVerification(false, "summary-only", receiptURA, invocationID, "summary projection is not cryptographic evidence", Map.of("profile", ReceiptSupport.PROFILE));
  }
}
