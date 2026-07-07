package run.easynet.daemon;

import java.util.List;
import java.util.Map;
import java.util.Objects;

public record ReceiptChainVerification(
    boolean verified,
    String method,
    String rootReceiptURA,
    String terminalReceiptURA,
    List<Object> items,
    Map<String, Object> metadata) {
  public ReceiptChainVerification {
    method = ReceiptSupport.required(method, "method");
    rootReceiptURA = rootReceiptURA == null ? "" : rootReceiptURA;
    terminalReceiptURA = terminalReceiptURA == null ? "" : terminalReceiptURA;
    items = items == null ? List.of() : List.copyOf(items);
    metadata = metadata == null ? Map.of() : Map.copyOf(metadata);
  }

  public static ReceiptChainVerification fromJSON(byte[] raw) {
    Objects.requireNonNull(raw, "raw");
    Map<String, Object> fields = JsonValueReader.object(raw, "receipt chain verification JSON");
    Object rawItems = fields.get("items");
    if (!(rawItems instanceof List<?> items)) {
      throw ReceiptSupport.invalid("items must be an array");
    }
    return new ReceiptChainVerification(
        ReceiptSupport.requiredBoolean(fields, "verified"),
        ReceiptSupport.requiredJSON(fields, "method"),
        ReceiptSupport.optionalJSON(fields, "root_receipt_ura"),
        ReceiptSupport.optionalJSON(fields, "terminal_receipt_ura"),
        List.copyOf(items),
        ReceiptSupport.optionalObject(fields, "metadata"));
  }
}
